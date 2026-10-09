//! `holdshort twin`: compacted positions + METAR → flights, events, runway bins.

use anyhow::bail;
use chrono::{Datelike, NaiveDate};
use holdshort_core::airport::Airport;
use holdshort_core::{Line, Metar, Obs, from_ndjson};
use holdshort_twin::classify::Kind;
use holdshort_twin::day::{EventKind, twin_day};

use crate::compact::obs_key;
use crate::parquet_io::{bins_parquet, events_parquet, flights_parquet, read_obs};
use crate::store::Store;
use crate::weather::month_key;

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 86_400_000;

#[derive(Debug, Default)]
pub struct TwinSummary {
    pub flights: usize,
    pub arrivals: usize,
    pub departures: usize,
    pub both: usize,
    pub overflights: usize,
    pub other: usize,
    pub go_arounds: usize,
    pub holds: usize,
    pub config_changes: usize,
}

fn start_ms(day: NaiveDate) -> i64 {
    day.and_hms_opt(0, 0, 0)
        .expect("midnight")
        .and_utc()
        .timestamp_millis()
}

fn read_day(store: &Store, ap: &Airport, day: NaiveDate) -> anyhow::Result<Option<Vec<Obs>>> {
    let key = obs_key(ap, day);
    if !store.exists(&key)? {
        return Ok(None);
    }
    Ok(Some(read_obs(store.get(&key)?)?))
}

fn read_metars(store: &Store, ap: &Airport, day: NaiveDate) -> anyhow::Result<Vec<Metar>> {
    let mut days = vec![day];
    // At a month edge, the neighbouring month's file holds the reports
    // just before or after midnight.
    if day.day() == 1 {
        days.extend(day.pred_opt());
    }
    if day.succ_opt().is_some_and(|n| n.month() != day.month()) {
        days.extend(day.succ_opt());
    }
    let mut out = Vec::new();
    for d in days {
        let key = month_key(ap, start_ms(d));
        if store.exists(&key)? {
            let text = String::from_utf8(store.get(&key)?)?;
            out.extend(from_ndjson(&text)?.into_iter().filter_map(|l| match l {
                Line::Metar(m) => Some(m),
                _ => None,
            }));
        }
    }
    Ok(out)
}

/// Run the twin for one airport-day and write its three Parquet files.
pub fn run_day(store: &Store, ap: &'static Airport, day: NaiveDate) -> anyhow::Result<TwinSummary> {
    let start = start_ms(day);
    let Some(mut obs) = read_day(store, ap, day)? else {
        bail!("no compacted data for {} {day}", ap.icao);
    };
    // Edge hours of the neighbouring days keep flights across midnight whole.
    for (d, keep) in [
        (day.pred_opt(), start - HOUR_MS..start),
        (day.succ_opt(), start + DAY_MS..start + DAY_MS + HOUR_MS),
    ] {
        if let Some(extra) = d.map(|d| read_day(store, ap, d)).transpose()?.flatten() {
            obs.extend(extra.into_iter().filter(|o| keep.contains(&o.ts_ms)));
        }
    }
    let twin = twin_day(ap, start, obs, &read_metars(store, ap, day)?);
    let dir = format!("{}/twin/date={day}", ap.icao);
    store.put(
        &format!("{dir}/flights.parquet"),
        flights_parquet(&twin.flights)?,
    )?;
    store.put(
        &format!("{dir}/events.parquet"),
        events_parquet(&twin.events)?,
    )?;
    store.put(
        &format!("{dir}/runway_bins.parquet"),
        bins_parquet(&twin.bins)?,
    )?;

    let mut s = TwinSummary {
        flights: twin.flights.len(),
        ..Default::default()
    };
    for f in &twin.flights {
        match f.kind {
            Kind::Arrival => s.arrivals += 1,
            Kind::Departure => s.departures += 1,
            Kind::Both => s.both += 1,
            Kind::Overflight => s.overflights += 1,
            Kind::Other => s.other += 1,
        }
        s.go_arounds += f.go_arounds as usize;
        s.holds += f.holds as usize;
    }
    s.config_changes = twin
        .events
        .iter()
        .filter(|e| e.kind == EventKind::ConfigChange)
        .count();
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parquet_io::obs_parquet;
    use holdshort_core::airport::LTFM;
    use holdshort_core::{Obs, Src};
    use holdshort_twin::geo::{NM_M, from_runway};

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 8).unwrap()
    }

    /// A 3° approach to LTFM 35L from 10 nm, 5 s steps, at 01:00 on `day()`.
    fn approach() -> Vec<Obs> {
        let r = LTFM.runways.iter().find(|r| r.name == "35L").unwrap();
        let t0 = 1_791_417_600_000 + 3_600_000;
        (0..=60)
            .map(|i| {
                let along = -10.0 * NM_M + i as f64 * 288.0;
                let (lat, lon) = from_runway(r, along, 0.0);
                let alt = r.elevation_ft + (-along * 3f64.to_radians().tan()) / 0.3048;
                Obs {
                    ts_ms: t0 + i * 5_000,
                    hex: "aaaaaa".into(),
                    flight: Some("THY1".into()),
                    reg: None,
                    ac_type: None,
                    lat,
                    lon,
                    alt_baro_ft: Some(alt as i32),
                    alt_geom_ft: Some(alt as i32),
                    on_ground: false,
                    gs_kt: Some(140.0),
                    track_deg: Some(r.heading_deg as f32),
                    vrate_fpm: Some(-740),
                    squawk: None,
                    category: None,
                    src: Src::History,
                }
            })
            .collect()
    }

    fn store_with_day() -> Store {
        let store = Store::memory();
        store
            .put(&obs_key(&LTFM, day()), obs_parquet(&approach()).unwrap())
            .unwrap();
        store
    }

    #[test]
    fn missing_neighbour_days_are_fine() {
        let s = run_day(&store_with_day(), &LTFM, day()).unwrap();
        assert_eq!((s.flights, s.arrivals), (1, 1));
    }

    #[test]
    fn writes_three_files() {
        let store = store_with_day();
        run_day(&store, &LTFM, day()).unwrap();
        for f in ["flights", "events", "runway_bins"] {
            assert!(
                store
                    .exists(&format!("LTFM/twin/date=2026-10-08/{f}.parquet"))
                    .unwrap(),
                "{f}"
            );
        }
    }

    #[test]
    fn day_without_obs_is_error() {
        let err = run_day(&Store::memory(), &LTFM, day())
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("no compacted data for LTFM 2026-10-08"),
            "{err}"
        );
    }
}
