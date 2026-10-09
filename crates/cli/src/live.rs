//! `holdshort live`: poll adsb.lol (OpenSky fallback) on a 10 s grid from
//! this machine and append to `data/raw/live/YYYY-MM-DD/HHMM.ndjson`.

use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use holdshort_core::area::retain_in_area;
use holdshort_core::ingest::{IngestState, chunk_key, next_poll_ms, slot_of};
use holdshort_core::{Line, Poll, Src, adsblol, metar, opensky, to_ndjson};
use serde::Deserialize;

use crate::UA;

const ADSBLOL_URL: &str = "https://api.adsb.lol/v2/point/41.2753/28.7519/50";
const AWC_URL: &str = "https://aviationweather.gov/api/data/metar?ids=LTFM&format=json&hours=1";
const TOKEN_URL: &str =
    "https://auth.opensky-network.org/auth/realms/opensky-network/protocol/openid-connect/token";

/// One poll: pick the feed, fetch, parse, keep the area, and record the
/// outcome. Returns the NDJSON lines to append (positions, one status line,
/// and METAR reports when due). Network calls are passed in so the rules
/// can be tested without the network.
pub fn poll_once(
    st: &mut IngestState,
    now: i64,
    mut fetch: impl FnMut(Src) -> Result<String, String>,
    mut fetch_metar: impl FnMut() -> Result<String, String>,
) -> Vec<Line> {
    let src = st.choose(now);
    let started = Instant::now();
    let result = fetch(src).and_then(|body| {
        let parsed = match src {
            Src::Adsblol => adsblol::parse(&body, now),
            _ => opensky::parse(&body, now),
        };
        parsed.map_err(|e| e.to_string())
    });
    let latency_ms = started.elapsed().as_millis() as u32;
    let mut lines = Vec::new();
    let poll = match result {
        Ok(mut obs) => {
            retain_in_area(&mut obs);
            let count = obs.len() as u32;
            lines.extend(obs.into_iter().map(Line::Obs));
            Poll {
                ts_ms: now,
                src,
                ok: true,
                count,
                latency_ms,
                error: None,
            }
        }
        Err(e) => Poll {
            ts_ms: now,
            src,
            ok: false,
            count: 0,
            latency_ms,
            error: Some(e),
        },
    };
    st.record(src, poll.ok, now);
    lines.push(Line::Poll(poll));
    if st.metar_due(now) {
        // Set even on failure, so a down METAR service is not hammered.
        st.last_metar_ms = now;
        if let Ok(ms) = fetch_metar().and_then(|b| metar::parse_awc(&b).map_err(|e| e.to_string()))
        {
            lines.extend(ms.into_iter().map(Line::Metar));
        }
    }
    lines
}

/// Append lines to the slot's chunk file under `root`.
pub fn append_lines(root: &Path, slot_ms: i64, lines: &[Line]) -> std::io::Result<()> {
    let path = root.join(chunk_key(slot_ms));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    f.write_all(to_ndjson(lines).as_bytes())
}

/// `KEY=VALUE` pairs from a `.env` file; comments, blank and malformed lines
/// are skipped, surrounding quotes removed.
pub fn parse_env(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| {
            let v = v.trim();
            let v = v
                .strip_prefix('"')
                .and_then(|x| x.strip_suffix('"'))
                .or_else(|| v.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))
                .unwrap_or(v);
            (k.trim().to_string(), v.to_string())
        })
        .collect()
}

/// Load `.env` into the environment without overriding variables already set.
pub fn load_env(path: &Path) {
    if let Ok(text) = std::fs::read_to_string(path) {
        for (k, v) in parse_env(&text) {
            if std::env::var_os(&k).is_none() {
                // SAFETY: called once at startup, before any other thread exists.
                unsafe { std::env::set_var(k, v) };
            }
        }
    }
}

#[derive(Deserialize)]
struct TokenResp {
    access_token: String,
    expires_in: i64,
}

/// OpenSky with an OAuth client-credentials token, cached until a minute
/// before it expires. Anonymous when no credentials are set.
struct OpenSky {
    creds: Option<(String, String)>,
    token: Option<(String, i64)>,
}

impl OpenSky {
    fn from_env() -> Self {
        let creds = std::env::var("OPENSKY_CLIENT_ID")
            .ok()
            .zip(std::env::var("OPENSKY_CLIENT_SECRET").ok());
        Self { creds, token: None }
    }

    fn bearer(
        &mut self,
        client: &reqwest::blocking::Client,
        now: i64,
    ) -> Result<Option<String>, String> {
        let Some((id, secret)) = &self.creds else {
            return Ok(None);
        };
        if let Some((t, exp)) = &self.token
            && exp - 60_000 > now
        {
            return Ok(Some(t.clone()));
        }
        let resp: TokenResp = client
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", id.as_str()),
                ("client_secret", secret.as_str()),
            ])
            .timeout(Duration::from_secs(10))
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.json())
            .map_err(|e| format!("opensky token: {e}"))?;
        self.token = Some((resp.access_token.clone(), now + resp.expires_in * 1000));
        Ok(Some(resp.access_token))
    }

    fn fetch(&mut self, client: &reqwest::blocking::Client, now: i64) -> Result<String, String> {
        let (lamin, lomin, lamax, lomax) = opensky::BBOX;
        let url = format!(
            "https://opensky-network.org/api/states/all?lamin={lamin}&lomin={lomin}&lamax={lamax}&lomax={lomax}"
        );
        let mut req = client.get(url).timeout(Duration::from_secs(10));
        if let Some(t) = self.bearer(client, now)? {
            req = req.bearer_auth(t);
        }
        get(req)
    }
}

fn get(req: reqwest::blocking::RequestBuilder) -> Result<String, String> {
    let resp = req.send().map_err(|e| e.to_string())?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("http {}", status.as_u16()));
    }
    resp.text().map_err(|e| e.to_string())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after 1970")
        .as_millis() as i64
}

/// Poll forever (Ctrl-C to stop). Prints one summary line per 5-min slot.
pub fn run(root: &Path) -> anyhow::Result<()> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(UA)
        .build()?;
    let mut opensky = OpenSky::from_env();
    let mut st = IngestState::default();
    let (mut slot, mut polls, mut ok, mut last_count) = (0, 0, 0, 0);
    println!(
        "polling into {} (OpenSky fallback: {})",
        root.display(),
        if opensky.creds.is_some() {
            "authenticated"
        } else {
            "anonymous"
        }
    );
    loop {
        let now = now_ms();
        let wait = next_poll_ms(now, st.interval_ms()) - now;
        std::thread::sleep(Duration::from_millis(wait.max(0) as u64));
        let now = now_ms();
        let lines = poll_once(
            &mut st,
            now,
            |src| match src {
                Src::Adsblol => get(client.get(ADSBLOL_URL).timeout(Duration::from_secs(5))),
                _ => opensky.fetch(&client, now),
            },
            || get(client.get(AWC_URL).timeout(Duration::from_secs(10))),
        );
        for l in &lines {
            if let Line::Poll(p) = l {
                polls += 1;
                if p.ok {
                    ok += 1;
                    last_count = p.count;
                } else {
                    eprintln!(
                        "{:?} poll failed: {}",
                        p.src,
                        p.error.as_deref().unwrap_or("?")
                    );
                }
            }
        }
        append_lines(root, slot_of(now), &lines)?;
        if slot_of(now) != slot {
            if slot != 0 {
                println!(
                    "{}: {ok}/{polls} polls ok, {last_count} aircraft, source {:?}",
                    chunk_key(slot),
                    st.source
                );
            }
            (slot, polls, ok) = (slot_of(now), 0, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use holdshort_core::{Line, Src, from_ndjson};

    const NOW: i64 = 1_791_561_733_502;
    const SMALL: &str = include_str!("../../../fixtures/adsblol_small.json");
    const AWC: &str = include_str!("../../../fixtures/metar_awc.json");

    fn count(lines: &[Line]) -> (usize, Vec<Poll>, usize) {
        let obs = lines.iter().filter(|l| matches!(l, Line::Obs(_))).count();
        let polls = lines
            .iter()
            .filter_map(|l| match l {
                Line::Poll(p) => Some(p.clone()),
                _ => None,
            })
            .collect();
        let metars = lines.iter().filter(|l| matches!(l, Line::Metar(_))).count();
        (obs, polls, metars)
    }

    #[test]
    fn ok_poll_writes_obs_poll_and_metar() {
        let mut st = IngestState::default();
        let lines = poll_once(
            &mut st,
            NOW,
            |_| Ok(SMALL.to_string()),
            || Ok(AWC.to_string()),
        );
        let (obs, polls, metars) = count(&lines);
        assert_eq!(obs, 3);
        assert_eq!(polls.len(), 1);
        assert!(polls[0].ok && polls[0].count == 3 && polls[0].src == Src::Adsblol);
        assert!(metars > 0);
        assert_eq!(st.last_metar_ms, NOW);
    }

    #[test]
    fn metar_not_refetched_within_30_min() {
        let mut st = IngestState {
            last_metar_ms: NOW - 60_000,
            ..IngestState::default()
        };
        let lines = poll_once(
            &mut st,
            NOW,
            |_| Ok(SMALL.to_string()),
            || panic!("not due"),
        );
        assert_eq!(count(&lines).2, 0);
    }

    #[test]
    fn failures_fail_over_to_opensky() {
        let mut st = IngestState {
            last_metar_ms: NOW,
            ..IngestState::default()
        };
        let mut asked = Vec::new();
        for i in 0..3 {
            let lines = poll_once(
                &mut st,
                NOW + i * 10_000,
                |src| {
                    asked.push(src);
                    Err("http 429".to_string())
                },
                || unreachable!(),
            );
            let (_, polls, _) = count(&lines);
            assert!(!polls[0].ok);
            assert_eq!(polls[0].error.as_deref(), Some("http 429"));
        }
        assert_eq!(asked, vec![Src::Adsblol, Src::Adsblol, Src::Opensky]);
    }

    #[test]
    fn error_body_is_a_failed_poll() {
        let mut st = IngestState {
            last_metar_ms: NOW,
            ..IngestState::default()
        };
        let lines = poll_once(&mut st, NOW, |_| Ok("{}".to_string()), || unreachable!());
        assert!(!count(&lines).1[0].ok);
        assert_eq!(st.primary_failures, 1);
    }

    #[test]
    fn metar_failure_still_waits_30_min() {
        let mut st = IngestState::default();
        let lines = poll_once(
            &mut st,
            NOW,
            |_| Ok(SMALL.to_string()),
            || Err("timeout".into()),
        );
        assert_eq!(count(&lines).2, 0);
        assert_eq!(st.last_metar_ms, NOW);
    }

    #[test]
    fn append_goes_to_slot_file() {
        let dir = tempfile::tempdir().unwrap();
        let slot = 1_791_561_600_000;
        let poll = |n| {
            vec![Line::Poll(Poll {
                ts_ms: n,
                src: Src::Adsblol,
                ok: true,
                count: 0,
                latency_ms: 1,
                error: None,
            })]
        };
        append_lines(dir.path(), slot, &poll(1)).unwrap();
        append_lines(dir.path(), slot, &poll(2)).unwrap();
        let text =
            std::fs::read_to_string(dir.path().join("raw/live/2026-10-09/1600.ndjson")).unwrap();
        assert_eq!(from_ndjson(&text).unwrap().len(), 2);
    }

    #[test]
    fn parses_env_file() {
        let text = "# comment\nA=1\n\nB = \"two words\"\nnot a pair\nC='x=y'\n";
        assert_eq!(
            parse_env(text),
            vec![
                ("A".to_string(), "1".to_string()),
                ("B".to_string(), "two words".to_string()),
                ("C".to_string(), "x=y".to_string()),
            ]
        );
    }
}
