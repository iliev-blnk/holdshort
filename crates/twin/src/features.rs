//! Feature rows for the forecasting models (sub-project 4).
//!
//! Computed here, in the engine, so training (Python reads the rows from
//! Parquet) and serving (Rust) use the same code.

use std::collections::HashMap;
use std::f64::consts::TAU;

use holdshort_core::Obs;
use holdshort_core::airport::Airport;
use holdshort_core::area::{bearing, distance_nm};

use crate::classify::Kind;
use crate::config::Bin;
use crate::day::{DayTwin, EventKind, Flight};
use crate::height::Weather;

const DAY_MS: i64 = 86_400_000;
const MIN_MS: i64 = 60_000;
const BIN_MS: i64 = 600_000;

pub const ETA_N: usize = 18;

/// Column order of `EtaRow::x`; the models' input order (see models/meta.json).
pub const ETA_FEATURES: [&str; ETA_N] = [
    "brg_sin",
    "brg_cos",
    "alt_ft",
    "gs_kt",
    "vrate_fpm",
    "trk_off_sin",
    "trk_off_cos",
    "hour_sin",
    "hour_cos",
    "dow",
    "inbound",
    "landed_15",
    "landed_60",
    "holding",
    "arr_rwys",
    "south",
    "wind_u",
    "wind_v",
];

/// One arrival as seen when it first comes within the ASMA ring (40 nm).
#[derive(Debug, Clone, PartialEq)]
pub struct EtaRow {
    pub flight_id: String,
    /// Ring entry, ms since epoch.
    pub t0_ms: i64,
    pub arr_rwy: Option<String>,
    /// 45° sector of the entry bearing from the airport, 0 = north.
    pub sector: u8,
    /// Great-circle distance from the entry point to the airport.
    pub dist_nm: f64,
    /// Target: entry → touchdown, seconds.
    pub asma_s: f64,
    /// Features in `ETA_FEATURES` order; NaN when unknown.
    pub x: [f32; ETA_N],
}

fn is_arrival(f: &Flight) -> bool {
    matches!(f.kind, Kind::Arrival | Kind::Both)
}

/// Configuration label in effect at `t_ms`: the latest runway bin that ended by then.
pub fn config_at(bins: &[Bin], t_ms: i64) -> &str {
    bins.iter()
        .filter(|b| b.bin_ms + BIN_MS <= t_ms)
        .max_by_key(|b| b.bin_ms)
        .map_or("", |b| b.config.as_str())
}

/// (number of arrival runways, 1.0 south flow / 0.0 north flow / NaN unknown)
/// from a label such as `ARR 34L,35R / DEP -`.
pub fn parse_config(label: &str) -> (u8, f32) {
    let Some(arr) = label
        .strip_prefix("ARR ")
        .and_then(|s| s.split(" /").next())
        .filter(|s| !s.is_empty() && *s != "-")
    else {
        return (0, f32::NAN);
    };
    let rwys: Vec<&str> = arr.split(',').collect();
    let number: u32 = rwys[0]
        .trim_end_matches(|c: char| c.is_ascii_alphabetic())
        .parse()
        .unwrap_or(0);
    let south = match number {
        16..=18 => 1.0,
        34..=36 => 0.0,
        _ => f32::NAN,
    };
    (rwys.len() as u8, south)
}

fn opt(v: Option<f64>) -> f32 {
    v.map_or(f32::NAN, |v| v as f32)
}

/// Feature rows for every arrival with a ring entry and a confident touchdown.
pub fn eta_rows(ap: &Airport, twin: &DayTwin, obs: &[Obs], wx: &Weather) -> Vec<EtaRow> {
    let mut by_hex: HashMap<&str, Vec<&Obs>> = HashMap::new();
    for o in obs {
        by_hex.entry(o.hex.as_str()).or_default().push(o);
    }
    for v in by_hex.values_mut() {
        v.sort_by_key(|o| o.ts_ms);
    }
    let mut touchdowns: Vec<i64> = twin
        .flights
        .iter()
        .filter(|f| is_arrival(f))
        .filter_map(|f| f.touchdown_ms)
        .collect();
    touchdowns.sort_unstable();
    let landed_in = |from: i64, to: i64| {
        (touchdowns.partition_point(|&t| t <= to) - touchdowns.partition_point(|&t| t <= from))
            as f32
    };
    // Holding intervals per flight: each start runs to the next end (or forever).
    let mut holds: Vec<(i64, i64)> = Vec::new();
    let mut open: HashMap<&str, i64> = HashMap::new();
    let mut evs: Vec<_> = twin.events.iter().collect();
    evs.sort_by_key(|e| e.ms);
    for e in evs {
        let Some(id) = e.flight_id.as_deref() else {
            continue;
        };
        match e.kind {
            EventKind::HoldStart => {
                open.insert(id, e.ms);
            }
            EventKind::HoldEnd => {
                if let Some(s) = open.remove(id) {
                    holds.push((s, e.ms));
                }
            }
            _ => {}
        }
    }
    holds.extend(open.values().map(|&s| (s, i64::MAX)));

    let mut rows = Vec::new();
    for f in &twin.flights {
        let (Some(t0), Some(_), Some(asma_s)) = (f.asma_entry_ms, f.touchdown_ms, f.asma_s) else {
            continue;
        };
        if !is_arrival(f) || f.time_low_confidence {
            continue;
        }
        let Some(p) = by_hex
            .get(f.hex.as_str())
            // The last point at or before the crossing: nothing seen later may leak in.
            // (The ring crossing is interpolated from a point outside, so one exists.)
            .and_then(|v| v.iter().rev().find(|o| o.ts_ms <= t0))
        else {
            continue;
        };
        let brg = bearing(ap.lat, ap.lon, p.lat, p.lon);
        let to_ap = bearing(p.lat, p.lon, ap.lat, ap.lon);
        let off = p.track_deg.map(|t| (t as f64 - to_ap).to_radians());
        let frac = t0.rem_euclid(DAY_MS) as f64 / DAY_MS as f64;
        // 1970-01-01 was a Thursday (Monday = 0).
        let dow = ((t0.div_euclid(DAY_MS) + 3).rem_euclid(7)) as f32;
        let inbound = twin
            .flights
            .iter()
            .filter(|o| o.id != f.id && is_arrival(o))
            .filter(|o| matches!((o.asma_entry_ms, o.touchdown_ms), (Some(e), Some(t)) if e <= t0 && t0 < t))
            .count() as f32;
        let holding = holds.iter().filter(|(s, e)| *s <= t0 && t0 < *e).count() as f32;
        let (arr_rwys, south) = parse_config(config_at(&twin.bins, t0));
        let (wind_u, wind_v) = match wx.at(t0) {
            Some(m) => match (m.wind_dir_deg, m.wind_kt) {
                (Some(d), Some(s)) => {
                    let r = d.to_radians();
                    ((-s * r.sin()) as f32, (-s * r.cos()) as f32)
                }
                _ => (f32::NAN, f32::NAN),
            },
            None => (f32::NAN, f32::NAN),
        };
        let x = [
            brg.to_radians().sin() as f32,
            brg.to_radians().cos() as f32,
            opt(p.alt_baro_ft.or(p.alt_geom_ft).map(f64::from)),
            opt(p.gs_kt.map(f64::from)),
            opt(p.vrate_fpm.map(f64::from)),
            opt(off.map(f64::sin)),
            opt(off.map(f64::cos)),
            (frac * TAU).sin() as f32,
            (frac * TAU).cos() as f32,
            dow,
            inbound,
            landed_in(t0 - 15 * MIN_MS, t0),
            landed_in(t0 - 60 * MIN_MS, t0),
            holding,
            arr_rwys as f32,
            south,
            wind_u,
            wind_v,
        ];
        rows.push(EtaRow {
            flight_id: f.id.clone(),
            t0_ms: t0,
            arr_rwy: f.arr_runway.clone(),
            sector: ((brg / 45.0).floor() as i64).rem_euclid(8) as u8,
            dist_nm: distance_nm(p.lat, p.lon, ap.lat, ap.lon),
            asma_s,
            x,
        });
    }
    rows
}

/// Touchdowns per UTC hour of the day starting at `day_start_ms`.
pub fn hourly_counts(twin: &DayTwin, day_start_ms: i64) -> [u32; 24] {
    let mut c = [0; 24];
    for t in twin
        .flights
        .iter()
        .filter(|f| is_arrival(f))
        .filter_map(|f| f.touchdown_ms)
    {
        let h = (t - day_start_ms).div_euclid(3_600_000);
        if (0..24).contains(&h) {
            c[h as usize] += 1;
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::Kind;
    use crate::config::Bin;
    use crate::day::{DayTwin, Event, EventKind, Flight};
    use crate::height::Weather;
    use crate::testkit::point;
    use holdshort_core::airport::LTFM;
    use holdshort_core::{Metar, Obs};

    /// 2026-10-02 00:00 UTC, a Friday.
    const DAY: i64 = 1_790_899_200_000;
    const MIN: i64 = 60_000;

    fn at(h: i64, m: i64) -> i64 {
        DAY + (h * 60 + m) * MIN
    }

    /// An arrival entering the ring at `entry`, landing at `td`.
    fn arrival(id: &str, hex: &str, entry: i64, td: i64) -> Flight {
        let mut f = Flight::empty(id, entry - 10 * MIN);
        f.hex = hex.into();
        f.last_ms = td;
        f.kind = Kind::Arrival;
        f.asma_entry_ms = Some(entry);
        f.touchdown_ms = Some(td);
        f.asma_s = Some((td - entry) as f64 / 1000.0);
        f.arr_runway = Some("35R".into());
        f
    }

    /// A point 40 nm due south of LTFM heading north at `t`.
    fn south_entry(hex: &str, t: i64) -> Obs {
        let mut p = point(
            t,
            LTFM.lat - 40.0 / 60.0,
            LTFM.lon,
            9000.0,
            280.0,
            0.0,
            -1500.0,
        );
        p.hex = hex.into();
        p
    }

    fn event(flight: Option<&str>, ms: i64, kind: EventKind) -> Event {
        Event {
            flight_id: flight.map(Into::into),
            ms,
            kind,
            runway: None,
            lat: None,
            lon: None,
            h_agl_ft: None,
            detail: None,
        }
    }

    fn bin(ms: i64, config: &str) -> Bin {
        Bin {
            bin_ms: ms,
            runway: "35R".into(),
            arrivals: 1,
            departures: 0,
            config: config.into(),
        }
    }

    fn twin(flights: Vec<Flight>, events: Vec<Event>, bins: Vec<Bin>) -> DayTwin {
        DayTwin {
            flights,
            events,
            bins,
        }
    }

    fn metar(ts: i64, raw: &str) -> Metar {
        Metar {
            station: "LTFM".into(),
            obs_ts_ms: ts,
            raw: raw.into(),
        }
    }

    fn col(name: &str) -> usize {
        ETA_FEATURES.iter().position(|f| *f == name).unwrap()
    }

    fn one(tw: &DayTwin, obs: &[Obs], wx: &Weather) -> EtaRow {
        let rows = eta_rows(&LTFM, tw, obs, wx);
        assert_eq!(rows.len(), 1, "expected one row");
        rows.into_iter().next().unwrap()
    }

    #[test]
    fn eta_row_basic() {
        let tw = twin(
            vec![arrival("a-1", "aaa", at(6, 0), at(6, 16))],
            vec![],
            vec![],
        );
        let wx = Weather::new(&[metar(
            at(5, 50),
            "METAR LTFM 020550Z 36010KT CAVOK 15/05 Q1015",
        )]);
        let r = one(&tw, &[south_entry("aaa", at(6, 0))], &wx);
        assert_eq!(r.flight_id, "a-1");
        assert_eq!(r.asma_s, 960.0);
        assert_eq!(r.t0_ms, at(6, 0));
        assert_eq!(r.arr_rwy.as_deref(), Some("35R"));
        // Due south: bearing 180°, sector 4.
        assert_eq!(r.sector, 4);
        assert!((r.dist_nm - 40.0).abs() < 0.2, "{}", r.dist_nm);
        assert!((r.x[col("brg_cos")] + 1.0).abs() < 1e-3);
        assert!(r.x[col("brg_sin")].abs() < 1e-2);
        // Heading north, straight at the airport.
        assert!((r.x[col("trk_off_cos")] - 1.0).abs() < 1e-3);
        assert_eq!(r.x[col("alt_ft")], 9000.0);
        assert_eq!(r.x[col("gs_kt")], 280.0);
        assert_eq!(r.x[col("vrate_fpm")], -1500.0);
        // 06:00 is a quarter of the day.
        assert!((r.x[col("hour_sin")] - 1.0).abs() < 1e-5);
        assert!(r.x[col("hour_cos")].abs() < 1e-5);
        assert_eq!(r.x[col("dow")], 4.0);
        // Wind from 360° at 10 kt blows towards the south: u = 0, v = -10.
        assert!(r.x[col("wind_u")].abs() < 1e-4);
        assert!((r.x[col("wind_v")] + 10.0).abs() < 1e-4);
    }

    #[test]
    fn inbound_counts_other_airborne_arrivals() {
        let flights = vec![
            arrival("a-1", "aaa", at(6, 0), at(6, 16)),
            // Entered earlier, still airborne at 06:00: inbound.
            arrival("b-1", "bbb", at(5, 50), at(6, 5)),
            // Already landed before 06:00: not inbound.
            arrival("c-1", "ccc", at(5, 30), at(5, 55)),
            // Enters later: not inbound.
            arrival("d-1", "ddd", at(6, 3), at(6, 20)),
        ];
        let rows = eta_rows(
            &LTFM,
            &twin(flights, vec![], vec![]),
            &[south_entry("aaa", at(6, 0))],
            &Weather::new(&[]),
        );
        let r = rows.iter().find(|r| r.flight_id == "a-1").unwrap();
        assert_eq!(r.x[col("inbound")], 1.0);
    }

    #[test]
    fn landed_windows() {
        let flights = vec![
            arrival("a-1", "aaa", at(6, 0), at(6, 16)),
            arrival("b-1", "bbb", at(5, 40), at(5, 50)),
            arrival("c-1", "ccc", at(5, 0), at(5, 20)),
            arrival("d-1", "ddd", at(4, 30), at(4, 50)),
        ];
        let rows = eta_rows(
            &LTFM,
            &twin(flights, vec![], vec![]),
            &[south_entry("aaa", at(6, 0))],
            &Weather::new(&[]),
        );
        let r = rows.iter().find(|r| r.flight_id == "a-1").unwrap();
        assert_eq!(r.x[col("landed_15")], 1.0);
        assert_eq!(r.x[col("landed_60")], 2.0);
    }

    #[test]
    fn holding_counts_open_holds() {
        let events = vec![
            event(Some("h-1"), at(5, 50), EventKind::HoldStart),
            event(Some("h-2"), at(5, 40), EventKind::HoldStart),
            event(Some("h-2"), at(5, 55), EventKind::HoldEnd),
            event(Some("h-3"), at(6, 5), EventKind::HoldStart),
        ];
        let tw = twin(
            vec![arrival("a-1", "aaa", at(6, 0), at(6, 16))],
            events,
            vec![],
        );
        let r = one(&tw, &[south_entry("aaa", at(6, 0))], &Weather::new(&[]));
        assert_eq!(r.x[col("holding")], 1.0);
    }

    #[test]
    fn parse_config_cases() {
        assert_eq!(parse_config("ARR 34L,35R / DEP -"), (2, 0.0));
        assert_eq!(parse_config("ARR 16R / DEP 17L").0, 1);
        assert_eq!(parse_config("ARR 16R / DEP 17L").1, 1.0);
        let (n, s) = parse_config("ARR - / DEP 35L");
        assert_eq!(n, 0);
        assert!(s.is_nan());
        let (n, s) = parse_config("");
        assert_eq!(n, 0);
        assert!(s.is_nan());
    }

    #[test]
    fn config_at_uses_bins_ended_before_t() {
        let bins = vec![
            bin(at(5, 40), "ARR 34L / DEP -"),
            bin(at(5, 50), "ARR 34L,35R / DEP -"),
        ];
        // The 05:50 bin ends at 06:00.
        assert_eq!(config_at(&bins, at(5, 59)), "ARR 34L / DEP -");
        assert_eq!(config_at(&bins, at(6, 0)), "ARR 34L,35R / DEP -");
        assert_eq!(config_at(&bins, at(5, 0)), "");
        let tw = twin(
            vec![arrival("a-1", "aaa", at(6, 0), at(6, 16))],
            vec![],
            bins,
        );
        let r = one(&tw, &[south_entry("aaa", at(6, 0))], &Weather::new(&[]));
        assert_eq!(r.x[col("arr_rwys")], 2.0);
        assert_eq!(r.x[col("south")], 0.0);
    }

    #[test]
    fn eta_row_without_metar_has_nan_wind() {
        let tw = twin(
            vec![arrival("a-1", "aaa", at(6, 0), at(6, 16))],
            vec![],
            vec![],
        );
        let r = one(&tw, &[south_entry("aaa", at(6, 0))], &Weather::new(&[]));
        assert!(r.x[col("wind_u")].is_nan());
        assert!(r.x[col("wind_v")].is_nan());
        let vrb = Weather::new(&[metar(
            at(5, 50),
            "METAR LTFM 020550Z VRB03KT CAVOK 15/05 Q1015",
        )]);
        assert!(one(&tw, &[south_entry("aaa", at(6, 0))], &vrb).x[col("wind_u")].is_nan());
    }

    #[test]
    fn eta_row_missing_gs_is_nan() {
        let tw = twin(
            vec![arrival("a-1", "aaa", at(6, 0), at(6, 16))],
            vec![],
            vec![],
        );
        let mut p = south_entry("aaa", at(6, 0));
        p.gs_kt = None;
        p.vrate_fpm = None;
        p.track_deg = None;
        let r = one(&tw, &[p], &Weather::new(&[]));
        assert!(r.x[col("gs_kt")].is_nan());
        assert!(r.x[col("vrate_fpm")].is_nan());
        assert!(r.x[col("trk_off_sin")].is_nan());
    }

    #[test]
    fn low_confidence_arrivals_are_skipped() {
        let mut a = arrival("a-1", "aaa", at(6, 0), at(6, 16));
        a.time_low_confidence = true;
        let mut dep = arrival("d-1", "ddd", at(6, 0), at(6, 16));
        dep.kind = Kind::Departure;
        let rows = eta_rows(
            &LTFM,
            &twin(vec![a, dep], vec![], vec![]),
            &[south_entry("aaa", at(6, 0)), south_entry("ddd", at(6, 0))],
            &Weather::new(&[]),
        );
        assert!(rows.is_empty());
    }

    #[test]
    fn eta_row_entered_before_midnight() {
        let tw = twin(
            vec![arrival("a-1", "aaa", DAY - 6 * MIN, DAY + 10 * MIN)],
            vec![],
            vec![],
        );
        let r = one(
            &tw,
            &[south_entry("aaa", DAY - 6 * MIN)],
            &Weather::new(&[]),
        );
        assert_eq!(r.asma_s, 960.0);
        // 23:54 is just before a full turn: hour_cos ≈ 1, hour_sin slightly negative.
        assert!(r.x[col("hour_cos")] > 0.99);
        assert!(r.x[col("hour_sin")] < 0.0);
        // Thursday.
        assert_eq!(r.x[col("dow")], 3.0);
    }

    #[test]
    fn entry_features_never_use_a_point_after_the_crossing() {
        let tw = twin(
            vec![arrival("a-1", "aaa", at(6, 0), at(6, 16))],
            vec![],
            vec![],
        );
        let before = south_entry("aaa", at(6, 0) - 5_000);
        let mut after = south_entry("aaa", at(6, 0) + 3_000);
        after.alt_baro_ft = Some(8_000);
        after.gs_kt = Some(250.0);
        let r = one(&tw, &[after, before], &Weather::new(&[]));
        assert_eq!(r.x[col("alt_ft")], 9000.0);
        assert_eq!(r.x[col("gs_kt")], 280.0);
    }

    #[test]
    fn hourly_counts_by_touchdown_hour() {
        let mut dep = arrival("d-1", "ddd", at(6, 0), at(6, 16));
        dep.kind = Kind::Departure;
        let flights = vec![
            arrival("a-1", "aaa", at(6, 0), at(6, 16)),
            arrival("b-1", "bbb", at(6, 30), at(6, 59)),
            arrival("c-1", "ccc", at(7, 0), at(7, 10)),
            // Lands just before midnight of the previous day: not counted.
            arrival("e-1", "eee", DAY - 30 * MIN, DAY - MIN),
            dep,
        ];
        let c = hourly_counts(&twin(flights, vec![], vec![]), DAY);
        assert_eq!(c[6], 2);
        assert_eq!(c[7], 1);
        assert_eq!(c.iter().sum::<u32>(), 3);
    }
}
