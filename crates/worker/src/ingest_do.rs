//! The `Ingest` Durable Object: one instance (`ltfm`) that polls the feeds on
//! an alarm loop and buffers rows in its SQLite storage.

use std::time::Duration;

use holdshort_core::area::retain_in_area;
use holdshort_core::ingest::{
    BUFFER_MS, Coverage, IngestState, chunk_key, coverage, overflow_slots, slot_of, slots_to_flush,
};
use holdshort_core::{Line, Poll, Src, adsblol, metar, opensky};
use serde::{Deserialize, Serialize};
use worker::*;

use crate::http;

const ADSBLOL_URL: &str = "https://api.adsb.lol/v2/point/41.2753/28.7519/50";
const AWC_URL: &str = "https://aviationweather.gov/api/data/metar?ids=LTFM&format=json&hours=1";
const ADSBLOL_TIMEOUT_MS: u32 = 5_000;
const OTHER_TIMEOUT_MS: u32 = 10_000;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS buf (slot INTEGER NOT NULL, line TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS buf_slot ON buf (slot);
CREATE TABLE IF NOT EXISTS polls (ts_ms INTEGER NOT NULL, ok INTEGER NOT NULL, error TEXT);
CREATE INDEX IF NOT EXISTS polls_ts ON polls (ts_ms);
CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v TEXT NOT NULL);
";

// `#[durable_object]` + `impl DurableObject` generate the JS class the
// runtime instantiates; the struct is our state between events.
#[durable_object]
pub struct Ingest {
    state: State,
    env: Env,
    sql: SqlStorage,
}

#[derive(Serialize)]
struct Health {
    source: Src,
    last_ok_ts: Option<i64>,
    buffered_rows: i64,
    windows_24h: Coverage,
    last_error: Option<String>,
}

#[derive(Deserialize)]
struct Ts {
    ts_ms: i64,
}

#[derive(Deserialize)]
struct Count {
    n: i64,
}

#[derive(Deserialize)]
struct MaybeTs {
    ts_ms: Option<i64>,
}

#[derive(Deserialize)]
struct ErrRow {
    error: Option<String>,
}

#[derive(Deserialize)]
struct MetaRow {
    v: String,
}

#[derive(Deserialize)]
struct SlotRow {
    slot: i64,
}

#[derive(Deserialize)]
struct LineRow {
    line: String,
}

#[derive(Serialize, Deserialize)]
struct Token {
    token: String,
    exp_ms: i64,
}

impl DurableObject for Ingest {
    fn new(state: State, env: Env) -> Self {
        let sql = state.storage().sql();
        sql.exec(SCHEMA, None).expect("create schema");
        Self { state, env, sql }
    }

    async fn fetch(&self, req: Request) -> Result<Response> {
        match (req.method(), req.path().as_str()) {
            (Method::Get, "/health") => Response::from_json(&self.health()?),
            (Method::Post, "/ensure") => {
                let armed = self.ensure().await?;
                Response::from_json(&serde_json::json!({ "armed": armed }))
            }
            _ => Response::error("Not found", 404),
        }
    }

    async fn alarm(&self) -> Result<Response> {
        let now = Date::now().as_millis() as i64;
        let mut st = self.load_state()?;
        // Re-arm first, so a failure below never stops the loop.
        self.arm(&st).await?;
        // `.await` yields to the runtime; nothing else runs inside this object meanwhile.
        self.poll(&mut st, now).await?;
        if st.metar_due(now) {
            st.last_metar_ms = now;
            self.poll_metar(now).await?;
        }
        self.drop_overflow(now)?;
        self.flush(now).await?;
        self.sql.exec(
            "DELETE FROM polls WHERE ts_ms <= ?",
            vec![(now - BUFFER_MS).into()],
        )?;
        self.save_state(&st)?;
        // The interval may have changed (failover); re-arm with the new one.
        self.arm(&st).await?;
        Response::ok("ok")
    }
}

impl Ingest {
    /// Set an alarm for now if none is set. Returns true if one was set.
    async fn ensure(&self) -> Result<bool> {
        let storage = self.state.storage();
        if storage.get_alarm().await?.is_some() {
            return Ok(false);
        }
        storage.set_alarm(Duration::ZERO).await?;
        Ok(true)
    }

    /// Schedule the next alarm. An `i64`/`Duration` alarm time is an offset from now.
    async fn arm(&self, st: &IngestState) -> Result<()> {
        let next = Duration::from_millis(st.interval_ms() as u64);
        self.state.storage().set_alarm(next).await
    }

    /// Poll the chosen feed once and buffer the result.
    async fn poll(&self, st: &mut IngestState, now: i64) -> Result<()> {
        let src = st.choose(now);
        let result = match src {
            Src::Adsblol => http::get_text(ADSBLOL_URL, &[], ADSBLOL_TIMEOUT_MS)
                .await
                .and_then(|b| adsblol::parse(&b, now).map_err(|e| e.to_string())),
            _ => self.poll_opensky(now).await,
        };
        let latency_ms = (Date::now().as_millis() as i64 - now) as u32;
        let slot = slot_of(now);
        let poll = match result {
            Ok(mut obs) => {
                retain_in_area(&mut obs);
                let count = obs.len() as u32;
                for o in obs {
                    self.buffer(slot, &Line::Obs(o))?;
                }
                Poll {
                    ts_ms: now,
                    src,
                    ok: true,
                    count,
                    latency_ms,
                    error: None,
                }
            }
            Err(e) => {
                console_error!("{src:?} poll failed: {e}");
                Poll {
                    ts_ms: now,
                    src,
                    ok: false,
                    count: 0,
                    latency_ms,
                    error: Some(e),
                }
            }
        };
        self.sql.exec(
            "INSERT INTO polls (ts_ms, ok, error) VALUES (?, ?, ?)",
            vec![
                now.into(),
                (poll.ok as i64).into(),
                poll.error.clone().unwrap_or_default().into(),
            ],
        )?;
        st.record(src, poll.ok, now);
        self.buffer(slot, &Line::Poll(poll))
    }

    async fn poll_opensky(
        &self,
        now: i64,
    ) -> std::result::Result<Vec<holdshort_core::Obs>, String> {
        let (lamin, lomin, lamax, lomax) = opensky::BBOX;
        let url = format!(
            "https://opensky-network.org/api/states/all?lamin={lamin}&lomin={lomin}&lamax={lamax}&lomax={lomax}"
        );
        let body = match self.opensky_token(now).await {
            Some(t) => {
                let auth = format!("Bearer {t}");
                http::get_text(&url, &[("Authorization", &auth)], OTHER_TIMEOUT_MS).await?
            }
            None => http::get_text(&url, &[], OTHER_TIMEOUT_MS).await?,
        };
        opensky::parse(&body, now).map_err(|e| e.to_string())
    }

    /// Cached OpenSky token; refreshed a minute before it expires.
    async fn opensky_token(&self, now: i64) -> Option<String> {
        let cached = self
            .meta("opensky_token")
            .and_then(|v| serde_json::from_str::<Token>(&v).ok())
            .filter(|t| t.exp_ms - 60_000 > now);
        if let Some(t) = cached {
            return Some(t.token);
        }
        match http::fetch_opensky_token(&self.env).await {
            Ok(Some((token, expires_in))) => {
                let t = Token {
                    token,
                    exp_ms: now + expires_in * 1000,
                };
                let _ = self.set_meta("opensky_token", &serde_json::to_string(&t).ok()?);
                Some(t.token)
            }
            Ok(None) => None,
            Err(e) => {
                console_error!("opensky token failed: {e}");
                None
            }
        }
    }

    async fn poll_metar(&self, now: i64) -> Result<()> {
        let parsed = http::get_text(AWC_URL, &[], OTHER_TIMEOUT_MS)
            .await
            .and_then(|b| metar::parse_awc(&b).map_err(|e| e.to_string()));
        match parsed {
            Ok(ms) => {
                for m in ms {
                    self.buffer(slot_of(now), &Line::Metar(m))?;
                }
            }
            Err(e) => console_error!("metar failed: {e}"),
        }
        Ok(())
    }

    fn buffer(&self, slot: i64, line: &Line) -> Result<()> {
        let json = serde_json::to_string(line)?;
        self.sql.exec(
            "INSERT INTO buf (slot, line) VALUES (?, ?)",
            vec![slot.into(), json.into()],
        )?;
        Ok(())
    }

    fn buffered_slots(&self) -> Result<Vec<i64>> {
        let rows: Vec<SlotRow> = self
            .sql
            .exec("SELECT DISTINCT slot FROM buf", None)?
            .to_array()?;
        Ok(rows.into_iter().map(|r| r.slot).collect())
    }

    /// Enforce the 24 h cap: drop the oldest slots and record the gap.
    fn drop_overflow(&self, now: i64) -> Result<()> {
        for slot in overflow_slots(&self.buffered_slots()?, now) {
            self.sql
                .exec("DELETE FROM buf WHERE slot = ?", vec![slot.into()])?;
            let gap = Poll {
                ts_ms: now,
                src: Src::Adsblol,
                ok: false,
                count: 0,
                latency_ms: 0,
                error: Some("buffer_overflow".into()),
            };
            self.buffer(slot_of(now), &Line::Poll(gap))?;
        }
        Ok(())
    }

    /// Write finished slots to R2, oldest first; stop at the first failure.
    async fn flush(&self, now: i64) -> Result<()> {
        let bucket = self.env.bucket("BUCKET")?;
        for slot in slots_to_flush(&self.buffered_slots()?, now) {
            let rows: Vec<LineRow> = self
                .sql
                .exec(
                    "SELECT line FROM buf WHERE slot = ? ORDER BY rowid",
                    vec![slot.into()],
                )?
                .to_array()?;
            let mut body = String::new();
            for r in rows {
                body.push_str(&r.line);
                body.push('\n');
            }
            if let Err(e) = bucket.put(chunk_key(slot), body).execute().await {
                console_error!("flush {slot} failed: {e}");
                break;
            }
            self.sql
                .exec("DELETE FROM buf WHERE slot = ?", vec![slot.into()])?;
        }
        Ok(())
    }

    fn meta(&self, k: &str) -> Option<String> {
        let rows: Vec<MetaRow> = self
            .sql
            .exec("SELECT v FROM meta WHERE k = ?", vec![k.into()])
            .ok()?
            .to_array()
            .ok()?;
        rows.into_iter().next().map(|r| r.v)
    }

    fn set_meta(&self, k: &str, v: &str) -> Result<()> {
        self.sql.exec(
            "INSERT OR REPLACE INTO meta (k, v) VALUES (?, ?)",
            vec![k.into(), v.into()],
        )?;
        Ok(())
    }

    fn save_state(&self, st: &IngestState) -> Result<()> {
        self.set_meta("state", &serde_json::to_string(st)?)
    }

    fn load_state(&self) -> Result<IngestState> {
        Ok(self
            .meta("state")
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default())
    }

    fn health(&self) -> Result<Health> {
        let now = Date::now().as_millis() as i64;
        let ok_ts: Vec<Ts> = self
            .sql
            .exec("SELECT ts_ms FROM polls WHERE ok = 1", None)?
            .to_array()?;
        let ok_ts: Vec<i64> = ok_ts.into_iter().map(|r| r.ts_ms).collect();
        let last_ok: MaybeTs = self
            .sql
            .exec("SELECT MAX(ts_ms) AS ts_ms FROM polls WHERE ok = 1", None)?
            .one()?;
        let buffered: Count = self
            .sql
            .exec("SELECT COUNT(*) AS n FROM buf", None)?
            .one()?;
        let last_error: Vec<ErrRow> = self
            .sql
            .exec(
                "SELECT error FROM polls WHERE ok = 0 ORDER BY ts_ms DESC LIMIT 1",
                None,
            )?
            .to_array()?;
        Ok(Health {
            source: self.load_state()?.source,
            last_ok_ts: last_ok.ts_ms,
            buffered_rows: buffered.n,
            windows_24h: coverage(&ok_ts, now),
            last_error: last_error.into_iter().next().and_then(|r| r.error),
        })
    }
}
