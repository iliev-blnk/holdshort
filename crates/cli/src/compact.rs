//! Merge one day's history + live data and write it as Parquet.

use std::collections::{BTreeSet, HashSet};
use std::io::Read;

use anyhow::bail;
use chrono::NaiveDate;
use flate2::read::GzDecoder;
use holdshort_core::airport::Airport;
use holdshort_core::{Line, Metar, Obs, Poll, from_ndjson};

use crate::backfill::history_key;
use crate::parquet_out::{obs_parquet, polls_parquet};
use crate::store::Store;
use crate::weather::store_metars;

const DAY_MS: i64 = 86_400_000;
const HOUR_MS: i64 = 3_600_000;

pub struct MergedDay {
    pub obs: Vec<Obs>,
    pub polls: Vec<Poll>,
    pub metars: Vec<Metar>,
}

#[derive(Debug)]
pub struct CompactStats {
    pub obs: usize,
    pub polls: usize,
    pub live_chunks: usize,
    pub had_history: bool,
}

fn day_start_ms(day: NaiveDate) -> i64 {
    day.and_hms_opt(0, 0, 0)
        .expect("midnight")
        .and_utc()
        .timestamp_millis()
}

/// Merge rules: keep only rows inside `day` (UTC); in any hour that has
/// history, history replaces live; dedupe `(hex, ts_ms)`, history first;
/// sort by `(ts_ms, hex)`.
pub fn merge_day(day: NaiveDate, history: Vec<Obs>, live: Vec<Line>) -> MergedDay {
    let start = day_start_ms(day);
    let in_day = |ts: i64| ts >= start && ts < start + DAY_MS;
    let history: Vec<Obs> = history.into_iter().filter(|o| in_day(o.ts_ms)).collect();
    let history_hours: HashSet<i64> = history
        .iter()
        .map(|o| o.ts_ms.div_euclid(HOUR_MS))
        .collect();
    let (mut live_obs, mut polls, mut metars) = (Vec::new(), Vec::new(), Vec::new());
    for line in live {
        match line {
            Line::Obs(o)
                if in_day(o.ts_ms) && !history_hours.contains(&o.ts_ms.div_euclid(HOUR_MS)) =>
            {
                live_obs.push(o)
            }
            Line::Obs(_) => {}
            Line::Poll(p) if in_day(p.ts_ms) => polls.push(p),
            Line::Poll(_) => {}
            Line::Metar(m) => metars.push(m),
        }
    }
    let mut seen: HashSet<(String, i64)> = HashSet::new();
    let mut obs: Vec<Obs> = history
        .into_iter()
        .chain(live_obs)
        .filter(|o| seen.insert((o.hex.clone(), o.ts_ms)))
        .collect();
    obs.sort_by(|a, b| (a.ts_ms, &a.hex).cmp(&(b.ts_ms, &b.hex)));
    polls.sort_by_key(|p| p.ts_ms);
    MergedDay { obs, polls, metars }
}

fn live_keys(store: &Store, airport: &Airport, day: NaiveDate) -> anyhow::Result<Vec<String>> {
    let icao = airport.icao;
    let mut keys: BTreeSet<String> = store
        .list(&format!("{icao}/raw/live/{day}/"))?
        .into_iter()
        .collect();
    // The next day's first slot (all its parts) can hold this day's last seconds.
    let next = day.succ_opt().expect("date in range");
    let first_slot = format!("{icao}/raw/live/{next}/0000");
    keys.extend(
        store
            .list(&format!("{icao}/raw/live/{next}/"))?
            .into_iter()
            .filter(|k| k.starts_with(&first_slot)),
    );
    Ok(keys.into_iter().collect())
}

pub fn obs_key(airport: &Airport, day: NaiveDate) -> String {
    format!("{}/parquet/date={day}/obs.parquet", airport.icao)
}

/// Compact one day into Parquet and merge its METAR into the monthly file.
pub fn compact_day(
    store: &Store,
    airport: &Airport,
    day: NaiveDate,
    force: bool,
) -> anyhow::Result<CompactStats> {
    if !force && store.exists(&obs_key(airport, day))? {
        bail!("{day} already compacted (use --force)");
    }
    let hkey = history_key(airport, day);
    let had_history = store.exists(&hkey)?;
    let history: Vec<Obs> = if had_history {
        let mut text = String::new();
        GzDecoder::new(store.get(&hkey)?.as_slice()).read_to_string(&mut text)?;
        from_ndjson(&text)?
            .into_iter()
            .filter_map(|l| match l {
                Line::Obs(o) => Some(o),
                _ => None,
            })
            .collect()
    } else {
        Vec::new()
    };
    let keys = live_keys(store, airport, day)?;
    if !had_history && keys.is_empty() {
        bail!("no data for {day}");
    }
    let mut live = Vec::new();
    for k in &keys {
        live.extend(from_ndjson(&String::from_utf8(store.get(k)?)?)?);
    }
    let merged = merge_day(day, history, live);
    store.put(&obs_key(airport, day), obs_parquet(&merged.obs)?)?;
    store.put(
        &format!("{}/parquet/date={day}/polls.parquet", airport.icao),
        polls_parquet(&merged.polls)?,
    )?;
    store_metars(store, airport, merged.metars)?;
    Ok(CompactStats {
        obs: merged.obs.len(),
        polls: merged.polls.len(),
        live_chunks: keys.len(),
        had_history,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use holdshort_core::airport::LTFM;
    use holdshort_core::{Src, to_ndjson};

    const D0: i64 = 1_791_417_600_000; // 2026-10-08 00:00 UTC
    const H: i64 = 3_600_000;

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 8).unwrap()
    }

    fn obs(hex: &str, ts_ms: i64, src: Src) -> Obs {
        Obs {
            ts_ms,
            hex: hex.into(),
            flight: None,
            reg: None,
            ac_type: None,
            lat: 41.27,
            lon: 28.75,
            alt_baro_ft: None,
            alt_geom_ft: None,
            on_ground: true,
            gs_kt: None,
            track_deg: None,
            vrate_fpm: None,
            squawk: None,
            category: None,
            src,
        }
    }

    fn poll(ts_ms: i64) -> Poll {
        Poll {
            ts_ms,
            src: Src::Adsblol,
            ok: true,
            count: 1,
            latency_ms: 100,
            error: None,
        }
    }

    #[test]
    fn history_wins_in_overlapping_hour() {
        let m = merge_day(
            day(),
            vec![obs("aaaaaa", D0 + 10 * H + 5_000, Src::History)],
            vec![Line::Obs(obs("bbbbbb", D0 + 10 * H + 7_000, Src::Adsblol))],
        );
        assert_eq!(m.obs.len(), 1);
        assert_eq!(m.obs[0].src, Src::History);
    }

    #[test]
    fn live_fills_missing_hour() {
        let m = merge_day(
            day(),
            vec![obs("aaaaaa", D0 + 10 * H, Src::History)],
            vec![Line::Obs(obs("bbbbbb", D0 + 11 * H, Src::Adsblol))],
        );
        assert_eq!(m.obs.len(), 2);
        assert_eq!(m.obs[1].hex, "bbbbbb");
    }

    #[test]
    fn dedupes_same_point() {
        let m = merge_day(
            day(),
            vec![],
            vec![
                Line::Obs(obs("aaaaaa", D0 + 1_000, Src::Adsblol)),
                Line::Obs(obs("aaaaaa", D0 + 1_000, Src::Adsblol)),
                Line::Obs(obs("aaaaaa", D0 + 2_000, Src::Adsblol)),
            ],
        );
        assert_eq!(m.obs.len(), 2);
    }

    #[test]
    fn output_sorted_by_time_then_hex() {
        let m = merge_day(
            day(),
            vec![],
            vec![
                Line::Obs(obs("bbbbbb", D0 + 2_000, Src::Adsblol)),
                Line::Obs(obs("cccccc", D0 + 1_000, Src::Adsblol)),
                Line::Obs(obs("aaaaaa", D0 + 2_000, Src::Adsblol)),
            ],
        );
        let order: Vec<&str> = m.obs.iter().map(|o| o.hex.as_str()).collect();
        assert_eq!(order, vec!["cccccc", "aaaaaa", "bbbbbb"]);
    }

    #[test]
    fn midnight_straggler_lands_in_previous_day() {
        let next = D0 + 24 * H;
        let m = merge_day(
            day(),
            vec![],
            vec![
                Line::Obs(obs("aaaaaa", next - 2_000, Src::Adsblol)),
                Line::Obs(obs("bbbbbb", next + 1_000, Src::Adsblol)),
                Line::Poll(poll(next + 5_000)),
                Line::Poll(poll(next - 5_000)),
            ],
        );
        assert_eq!(m.obs.len(), 1);
        assert_eq!(m.obs[0].hex, "aaaaaa");
        assert_eq!(m.polls.len(), 1, "polls are also cut to the day");
    }

    #[test]
    fn straggler_parts_included() {
        let store = Store::memory();
        for k in [
            "2026-10-09/0000.ndjson",
            "2026-10-09/0000-1.ndjson",
            "2026-10-09/0005.ndjson",
        ] {
            store
                .put(&format!("LTFM/raw/live/{k}"), b"".to_vec())
                .unwrap();
        }
        let keys = live_keys(&store, &LTFM, day()).unwrap();
        assert_eq!(
            keys,
            vec![
                "LTFM/raw/live/2026-10-09/0000-1.ndjson",
                "LTFM/raw/live/2026-10-09/0000.ndjson"
            ]
        );
    }

    #[test]
    fn compact_without_sources_errors() {
        let store = Store::memory();
        let err = compact_day(&store, &LTFM, day(), false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("no data for 2026-10-08"), "{err}");
    }

    #[test]
    fn compact_day_end_to_end() {
        let store = Store::memory();
        let live = to_ndjson(&[
            Line::Obs(obs("bbbbbb", D0 + 11 * H, Src::Adsblol)),
            Line::Poll(poll(D0 + 11 * H)),
            Line::Metar(Metar {
                station: "LTFM".into(),
                obs_ts_ms: D0 + 11 * H,
                raw: "LTFM X".into(),
            }),
        ]);
        store
            .put("LTFM/raw/live/2026-10-08/1100.ndjson", live.into_bytes())
            .unwrap();
        let straggler = to_ndjson(&[Line::Obs(obs("cccccc", D0 + 24 * H - 1_000, Src::Adsblol))]);
        store
            .put(
                "LTFM/raw/live/2026-10-09/0000.ndjson",
                straggler.into_bytes(),
            )
            .unwrap();
        let stats = compact_day(&store, &LTFM, day(), false).unwrap();
        assert_eq!(stats.obs, 2);
        assert_eq!(stats.polls, 1);
        assert!(
            store
                .exists("LTFM/parquet/date=2026-10-08/obs.parquet")
                .unwrap()
        );
        assert!(
            store
                .exists("LTFM/parquet/date=2026-10-08/polls.parquet")
                .unwrap()
        );
        let weather = String::from_utf8(store.get("LTFM/weather/2026-10.ndjson").unwrap()).unwrap();
        assert!(weather.contains("LTFM X"));
    }
}
