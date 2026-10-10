//! `holdshort export`: a twin day as static files for the console.
//!
//! `day.json` holds flights, events, the configuration sequence and the
//! movements histogram; `tracks.bin` holds every position as little-endian
//! columns the browser reads straight into typed arrays.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

#[cfg(test)]
use anyhow::{bail, ensure};
use chrono::NaiveDate;
use holdshort_core::Obs;
use holdshort_core::airport::Airport;
use holdshort_twin::day::DayTwin;
use serde::{Deserialize, Serialize};

use crate::store::Store;
use crate::twin::compute_day;

pub const MAGIC: &[u8; 4] = b"HSTK";
pub const VERSION: u32 = 1;
const HEADER: usize = 24;
const BIN_MS: i64 = 600_000;
const BINS: usize = 144;
const NO_ALT: i16 = i16::MIN;
const NO_U16: u16 = u16::MAX;

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct DayJson {
    pub v: u32,
    pub airport: String,
    pub date: String,
    pub day_start_ms: i64,
    pub flights: Vec<FlightJ>,
    pub events: Vec<EventJ>,
    pub config: Vec<ConfigJ>,
    pub hist: HistJ,
}

/// Times are ms since the day start.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct FlightJ {
    pub id: String,
    pub cs: Option<String>,
    #[serde(rename = "type")]
    pub ac_type: Option<String>,
    pub reg: Option<String>,
    pub kind: String,
    pub arr_rwy: Option<String>,
    pub td: Option<i64>,
    pub dep_rwy: Option<String>,
    pub lo: Option<i64>,
    pub low: bool,
    pub asma_s: Option<i64>,
    pub ga: u32,
    pub holds: u32,
    pub hold_s: i64,
    /// First point and point count in `tracks.bin`.
    pub p: [u32; 2],
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct EventJ {
    pub t: i64,
    pub kind: String,
    /// Index into `flights`; None for events without a flight.
    pub f: Option<u32>,
    pub rwy: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub detail: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct ConfigJ {
    pub t: i64,
    pub label: String,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct HistJ {
    pub bin_s: u32,
    pub arr: Vec<u32>,
    pub dep: Vec<u32>,
}

/// Decoded `tracks.bin` (used by tests and by `export` itself).
#[derive(Debug, Default, PartialEq)]
pub struct Tracks {
    pub day_start_ms: i64,
    pub t: Vec<i32>,
    pub lat: Vec<i32>,
    pub lon: Vec<i32>,
    pub alt: Vec<i16>,
    pub trk: Vec<u16>,
    pub gs: Vec<u16>,
}

fn serde_name<T: Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}

fn start_ms(day: NaiveDate) -> i64 {
    day.and_hms_opt(0, 0, 0)
        .expect("midnight")
        .and_utc()
        .timestamp_millis()
}

/// One day as `day.json` content and `tracks.bin` bytes.
pub fn build(airport: &str, day: NaiveDate, twin: &DayTwin, obs: &[Obs]) -> (DayJson, Vec<u8>) {
    let start = start_ms(day);
    let mut by_hex: HashMap<&str, Vec<&Obs>> = HashMap::new();
    for o in obs {
        by_hex.entry(o.hex.as_str()).or_default().push(o);
    }
    for v in by_hex.values_mut() {
        v.sort_by_key(|o| o.ts_ms);
    }

    let mut tr = Tracks {
        day_start_ms: start,
        ..Default::default()
    };
    let mut flights = Vec::with_capacity(twin.flights.len());
    let mut index = HashMap::new();
    for (i, f) in twin.flights.iter().enumerate() {
        index.insert(f.id.as_str(), i as u32);
        let pts: &[&Obs] = by_hex.get(f.hex.as_str()).map_or(&[], |v| v);
        let a = pts.partition_point(|o| o.ts_ms < f.first_ms);
        let b = pts.partition_point(|o| o.ts_ms <= f.last_ms);
        let first = tr.t.len() as u32;
        for o in &pts[a..b] {
            tr.t.push(((o.ts_ms - start) as f64 / 100.0).round() as i32);
            tr.lat.push((o.lat * 1e5).round() as i32);
            tr.lon.push((o.lon * 1e5).round() as i32);
            tr.alt.push(
                o.alt_baro_ft
                    .or(o.alt_geom_ft)
                    .map_or(NO_ALT, |a| a.clamp(-32767, 32767) as i16),
            );
            tr.trk.push(o.track_deg.map_or(NO_U16, |d| {
                ((d as f64).rem_euclid(360.0) * 100.0).round() as u16
            }));
            tr.gs.push(o.gs_kt.map_or(NO_U16, |g| {
                ((g as f64) * 10.0).round().clamp(0.0, 65534.0) as u16
            }));
        }
        flights.push(FlightJ {
            id: f.id.clone(),
            cs: f.flight.clone(),
            ac_type: f.ac_type.clone(),
            reg: f.reg.clone(),
            kind: serde_name(&f.kind),
            arr_rwy: f.arr_runway.clone(),
            td: f.touchdown_ms.map(|ms| ms - start),
            dep_rwy: f.dep_runway.clone(),
            lo: f.liftoff_ms.map(|ms| ms - start),
            low: f.time_low_confidence,
            asma_s: f.asma_s.map(|s| s.round() as i64),
            ga: f.go_arounds,
            holds: f.holds,
            hold_s: f.holding_s.round() as i64,
            p: [first, tr.t.len() as u32 - first],
        });
    }

    let events = twin
        .events
        .iter()
        .map(|e| EventJ {
            t: e.ms - start,
            kind: serde_name(&e.kind),
            f: e.flight_id.as_deref().and_then(|id| index.get(id).copied()),
            rwy: e.runway.clone(),
            lat: e.lat,
            lon: e.lon,
            detail: e.detail.clone(),
        })
        .collect();

    // One label per bin that has rows; consecutive equal labels collapse.
    let mut bins: Vec<(i64, &str)> = twin
        .bins
        .iter()
        .map(|b| (b.bin_ms, b.config.as_str()))
        .collect();
    bins.sort_by_key(|b| b.0);
    bins.dedup_by_key(|b| b.0);
    let mut config: Vec<ConfigJ> = Vec::new();
    for (ms, label) in bins {
        if config.last().is_none_or(|c| c.label != label) {
            config.push(ConfigJ {
                t: ms - start,
                label: label.into(),
            });
        }
    }

    let mut hist = HistJ {
        bin_s: (BIN_MS / 1000) as u32,
        arr: vec![0; BINS],
        dep: vec![0; BINS],
    };
    let count = |ms: Option<i64>, into: &mut Vec<u32>| {
        if let Some(ms) = ms {
            let b = (ms - start).div_euclid(BIN_MS);
            if (0..BINS as i64).contains(&b) {
                into[b as usize] += 1;
            }
        }
    };
    for f in &twin.flights {
        count(f.touchdown_ms, &mut hist.arr);
        count(f.liftoff_ms, &mut hist.dep);
    }

    let json = DayJson {
        v: 1,
        airport: airport.into(),
        date: day.to_string(),
        day_start_ms: start,
        flights,
        events,
        config,
        hist,
    };
    (json, encode_tracks(&tr))
}

fn encode_tracks(tr: &Tracks) -> Vec<u8> {
    let n = tr.t.len();
    let mut b = Vec::with_capacity(HEADER + 18 * n);
    b.extend_from_slice(MAGIC);
    b.extend_from_slice(&VERSION.to_le_bytes());
    b.extend_from_slice(&(n as u32).to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&tr.day_start_ms.to_le_bytes());
    for col in [&tr.t, &tr.lat, &tr.lon] {
        col.iter()
            .for_each(|v| b.extend_from_slice(&v.to_le_bytes()));
    }
    tr.alt
        .iter()
        .for_each(|v| b.extend_from_slice(&v.to_le_bytes()));
    for col in [&tr.trk, &tr.gs] {
        col.iter()
            .for_each(|v| b.extend_from_slice(&v.to_le_bytes()));
    }
    b
}

/// Write `<out>/<airport>/<date>/{day.json,tracks.bin.gz}`.
pub fn write_day(
    out: &Path,
    airport: &str,
    date: &str,
    json: &DayJson,
    tracks: &[u8],
) -> anyhow::Result<()> {
    let dir = out.join(airport).join(date);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("day.json"), serde_json::to_vec(json)?)?;
    // Gzipped here because static hosts don't compress application/octet-stream;
    // the console unpacks it with DecompressionStream.
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    gz.write_all(tracks)?;
    std::fs::write(dir.join("tracks.bin.gz"), gz.finish()?)?;
    Ok(())
}

/// Export one airport-day; returns a one-line summary.
pub fn run_day(
    store: &Store,
    ap: &'static Airport,
    day: NaiveDate,
    out: &Path,
) -> anyhow::Result<String> {
    let (twin, obs, _) = compute_day(store, ap, day)?;
    let (json, tracks) = build(ap.icao, day, &twin, &obs);
    write_day(out, ap.icao, &day.to_string(), &json, &tracks)?;
    let points = (tracks.len() - HEADER) / 18;
    Ok(format!(
        "{} flights, {points} points, {} KB",
        json.flights.len(),
        tracks.len() / 1024
    ))
}

#[derive(Serialize)]
struct IndexJ {
    v: u32,
    airport: String,
    days: Vec<DaySummaryJ>,
}

#[derive(Serialize)]
struct DaySummaryJ {
    date: String,
    arrivals: usize,
    departures: usize,
    go_arounds: u32,
    asma_median_s: Option<i64>,
}

/// Rebuild `<out>/index.json` from every exported day of `airport`.
pub fn write_index(out: &Path, airport: &str) -> anyhow::Result<usize> {
    let mut days = Vec::new();
    let dir = out.join(airport);
    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path().join("day.json");
        if !path.exists() {
            continue;
        }
        let j: DayJson = serde_json::from_slice(&std::fs::read(&path)?)?;
        let count = |kinds: [&str; 2]| {
            j.flights
                .iter()
                .filter(|f| kinds.contains(&f.kind.as_str()))
                .count()
        };
        let mut asma: Vec<i64> = j.flights.iter().filter_map(|f| f.asma_s).collect();
        asma.sort_unstable();
        days.push(DaySummaryJ {
            date: j.date.clone(),
            arrivals: count(["arrival", "both"]),
            departures: count(["departure", "both"]),
            go_arounds: j.flights.iter().map(|f| f.ga).sum(),
            asma_median_s: asma.get(asma.len() / 2).copied(),
        });
    }
    days.sort_by(|a, b| a.date.cmp(&b.date));
    let n = days.len();
    let idx = IndexJ {
        v: 1,
        airport: airport.into(),
        days,
    };
    std::fs::write(out.join("index.json"), serde_json::to_vec(&idx)?)?;
    Ok(n)
}

#[cfg(test)]
pub fn decode_tracks(bytes: &[u8]) -> anyhow::Result<Tracks> {
    ensure!(bytes.len() >= HEADER, "tracks.bin too short");
    if &bytes[0..4] != MAGIC {
        bail!("bad magic");
    }
    let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().expect("4 bytes"));
    let version = u32_at(4);
    ensure!(version == VERSION, "unsupported version {version}");
    let n = u32_at(8) as usize;
    ensure!(
        bytes.len() == HEADER + 18 * n,
        "tracks.bin size does not match n"
    );
    let day_start_ms = i64::from_le_bytes(bytes[16..24].try_into().expect("8 bytes"));
    let col = |at: usize, width: usize| bytes[at..at + width * n].chunks_exact(width);
    let i32s = |at| {
        col(at, 4)
            .map(|c| i32::from_le_bytes(c.try_into().expect("4")))
            .collect()
    };
    let u16s = |at| {
        col(at, 2)
            .map(|c| u16::from_le_bytes(c.try_into().expect("2")))
            .collect()
    };
    Ok(Tracks {
        day_start_ms,
        t: i32s(HEADER),
        lat: i32s(HEADER + 4 * n),
        lon: i32s(HEADER + 8 * n),
        alt: col(HEADER + 12 * n, 2)
            .map(|c| i16::from_le_bytes(c.try_into().expect("2")))
            .collect(),
        trk: u16s(HEADER + 14 * n),
        gs: u16s(HEADER + 16 * n),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use holdshort_core::{Obs, Src};
    use holdshort_twin::classify::Kind;
    use holdshort_twin::config::Bin;
    use holdshort_twin::day::{DayTwin, Event, EventKind, Flight};

    const START: i64 = 1_790_899_200_000;
    const MIN: i64 = 60_000;

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 2).unwrap()
    }

    #[allow(clippy::too_many_arguments)]
    fn obs(
        hex: &str,
        ts_ms: i64,
        lat: f64,
        lon: f64,
        alt: Option<i32>,
        geom: Option<i32>,
        trk: Option<f32>,
        gs: Option<f32>,
    ) -> Obs {
        Obs {
            ts_ms,
            hex: hex.into(),
            flight: None,
            reg: None,
            ac_type: None,
            lat,
            lon,
            alt_baro_ft: alt,
            alt_geom_ft: geom,
            on_ground: false,
            gs_kt: gs,
            track_deg: trk,
            vrate_fpm: None,
            squawk: None,
            category: None,
            src: Src::History,
        }
    }

    fn flight(id: &str, hex: &str, first: i64, last: i64, kind: Kind) -> Flight {
        let mut f = Flight::empty(id, first);
        f.hex = hex.into();
        f.last_ms = last;
        f.kind = kind;
        f
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

    fn twin(flights: Vec<Flight>, events: Vec<Event>, bins: Vec<Bin>) -> DayTwin {
        DayTwin {
            flights,
            events,
            bins,
        }
    }

    /// Two flights: "a" with 3 points, "b" with 2; plus a point of "a"
    /// outside its flight that must not be exported.
    fn two_flights() -> (DayTwin, Vec<Obs>) {
        let t0 = START + 6 * 60 * MIN;
        let o = vec![
            obs(
                "aaa",
                t0,
                41.0,
                28.5,
                Some(5000),
                None,
                Some(10.0),
                Some(250.0),
            ),
            obs(
                "bbb",
                t0 + 5_000,
                41.2,
                28.7,
                Some(3000),
                None,
                Some(359.5),
                Some(140.5),
            ),
            obs(
                "aaa",
                t0 + 10_000,
                41.00001,
                28.50002,
                Some(4800),
                None,
                Some(11.0),
                Some(249.0),
            ),
            obs(
                "aaa",
                t0 + 20_000,
                41.1234567,
                28.7654321,
                Some(4600),
                None,
                Some(12.0),
                Some(248.0),
            ),
            obs(
                "bbb",
                t0 + 15_000,
                41.21,
                28.71,
                Some(2900),
                None,
                Some(359.0),
                Some(140.0),
            ),
            obs(
                "aaa",
                t0 + 90 * MIN,
                42.0,
                29.0,
                Some(30000),
                None,
                None,
                None,
            ),
        ];
        let fl = vec![
            flight("aaa-1", "aaa", t0, t0 + 20_000, Kind::Overflight),
            flight("bbb-1", "bbb", t0 + 5_000, t0 + 15_000, Kind::Arrival),
        ];
        (twin(fl, vec![], vec![]), o)
    }

    #[test]
    fn tracks_round_trip() {
        let (tw, o) = two_flights();
        let (_, bytes) = build("LTFM", day(), &tw, &o);
        assert_eq!(bytes.len(), 24 + 18 * 5);
        let t = decode_tracks(&bytes).unwrap();
        assert_eq!(t.day_start_ms, START);
        assert_eq!(t.t.len(), 5);
        // Flight "aaa" first, in time order.
        assert_eq!(t.t[0], 6 * 3_600_000 / 100);
        assert_eq!(t.t[1], t.t[0] + 100);
        assert_eq!(t.lat[2], 4_112_346);
        assert_eq!(t.lon[2], 2_876_543);
        assert_eq!(t.alt[2], 4600);
        assert_eq!(t.trk[3], 35_950);
        assert_eq!(t.gs[3], 1405);
    }

    #[test]
    fn points_before_midnight_keep_negative_t() {
        let t0 = START - 30 * MIN;
        let o = vec![
            obs(
                "ccc",
                t0,
                41.0,
                28.5,
                Some(9000),
                None,
                Some(0.0),
                Some(300.0),
            ),
            obs(
                "ccc",
                START + MIN,
                41.1,
                28.5,
                Some(8000),
                None,
                Some(0.0),
                Some(300.0),
            ),
        ];
        let tw = twin(
            vec![flight("ccc-1", "ccc", t0, START + MIN, Kind::Other)],
            vec![],
            vec![],
        );
        let t = decode_tracks(&build("LTFM", day(), &tw, &o).1).unwrap();
        assert_eq!(t.t, vec![-18_000, 600]);
    }

    #[test]
    fn missing_values_use_markers() {
        let o = vec![
            obs("ddd", START, 41.0, 28.5, None, None, None, None),
            obs(
                "ddd",
                START + MIN,
                41.0,
                28.5,
                None,
                Some(7000),
                Some(90.0),
                Some(200.0),
            ),
        ];
        let tw = twin(
            vec![flight("ddd-1", "ddd", START, START + MIN, Kind::Other)],
            vec![],
            vec![],
        );
        let t = decode_tracks(&build("LTFM", day(), &tw, &o).1).unwrap();
        assert_eq!((t.alt[0], t.trk[0], t.gs[0]), (-32768, 65535, 65535));
        assert_eq!((t.alt[1], t.trk[1], t.gs[1]), (7000, 9000, 2000));
    }

    #[test]
    fn flight_slices_follow_flight_order() {
        let (tw, o) = two_flights();
        let (j, _) = build("LTFM", day(), &tw, &o);
        assert_eq!(j.flights[0].p, [0, 3]);
        assert_eq!(j.flights[1].p, [3, 2]);
        assert_eq!(j.flights[1].id, "bbb-1");
        assert_eq!(j.flights[1].kind, "arrival");
        assert_eq!(j.day_start_ms, START);
        assert_eq!(j.date, "2026-10-02");
    }

    #[test]
    fn events_point_at_flights() {
        let (mut tw, o) = two_flights();
        let td = START + 6 * 60 * MIN + 20_000;
        tw.events = vec![
            event(Some("bbb-1"), td, EventKind::Touchdown),
            event(None, START + 6 * 60 * MIN, EventKind::ConfigChange),
        ];
        let (j, _) = build("LTFM", day(), &tw, &o);
        assert_eq!(j.events[0].f, Some(1));
        assert_eq!(j.events[0].t, td - START);
        assert_eq!(j.events[0].kind, "touchdown");
        assert_eq!(j.events[1].f, None);
        assert_eq!(j.events[1].kind, "config_change");
    }

    #[test]
    fn config_collapses_equal_labels() {
        let bin = |m: i64, label: &str| Bin {
            bin_ms: START + m * MIN,
            runway: "35R".into(),
            arrivals: 1,
            departures: 0,
            config: label.into(),
        };
        let tw = twin(
            vec![],
            vec![],
            vec![bin(360, "A"), bin(370, "A"), bin(380, "B")],
        );
        let (j, _) = build("LTFM", day(), &tw, &[]);
        let got: Vec<(i64, &str)> = j.config.iter().map(|c| (c.t, c.label.as_str())).collect();
        assert_eq!(got, vec![(21_600_000, "A"), (22_800_000, "B")]);
    }

    #[test]
    fn hist_counts_departures_without_runway() {
        let mut dep = flight("dep-1", "eee", START, START + 400 * MIN, Kind::Departure);
        dep.liftoff_ms = Some(START + 365 * MIN);
        let mut arr = flight("arr-1", "fff", START, START + 400 * MIN, Kind::Arrival);
        arr.touchdown_ms = Some(START + 375 * MIN);
        arr.arr_runway = Some("35R".into());
        let mut late = flight("arr-2", "ggg", START, START + 1500 * MIN, Kind::Arrival);
        late.touchdown_ms = Some(START + 1445 * MIN);
        let (j, _) = build(
            "LTFM",
            day(),
            &twin(vec![dep, arr, late], vec![], vec![]),
            &[],
        );
        assert_eq!(j.hist.bin_s, 600);
        assert_eq!(j.hist.arr.len(), 144);
        assert_eq!(j.hist.dep[36], 1);
        assert_eq!(j.hist.arr[37], 1);
        assert_eq!(j.hist.arr.iter().sum::<u32>(), 1);
    }

    #[test]
    fn decode_rejects_bad_magic_and_version() {
        let (tw, o) = two_flights();
        let mut b = build("LTFM", day(), &tw, &o).1;
        b[0] = b'X';
        assert!(decode_tracks(&b).is_err());
        let mut b = build("LTFM", day(), &tw, &o).1;
        b[4] = 2;
        assert!(decode_tracks(&b).is_err());
    }

    /// The committed console fixture: one arrival to 35R, one departure
    /// first seen high (no runway), one overflight, one configuration.
    fn fixture_day() -> (DayTwin, Vec<Obs>) {
        let at = |h: i64, m: i64| START + (h * 60 + m) * MIN;
        let mut o = Vec::new();
        // THY1: 35R final, 06:02–06:12, 3,200 ft → 400 ft.
        for k in 0..11 {
            let nm = 10.0 - k as f64;
            let mut p = obs(
                "4bb208",
                at(6, 2) + k * MIN,
                41.26192 - nm / 60.0,
                28.72776,
                Some((400.0 + nm * 280.0) as i32),
                None,
                Some(359.0),
                Some(150.0),
            );
            p.flight = Some("THY1".into());
            o.push(p);
        }
        // PGT2: first seen at 9,000 ft, 06:09–06:18, climbing north.
        for k in 0..10 {
            let mut p = obs(
                "4ba9c1",
                at(6, 9) + k * MIN,
                41.40 + k as f64 * 0.06,
                28.74,
                Some(9000 + k as i32 * 1500),
                None,
                Some(2.0),
                Some(300.0),
            );
            p.flight = Some("PGT2".into());
            o.push(p);
        }
        // UAE3: overflight at FL360, west to east, 06:00–06:30.
        for k in 0..11 {
            let mut p = obs(
                "896180",
                at(6, 0) + k * 3 * MIN,
                41.60,
                28.00 + k as f64 * 0.14,
                Some(36000),
                None,
                Some(90.0),
                Some(480.0),
            );
            p.flight = Some("UAE3".into());
            o.push(p);
        }
        let mut thy = flight("4bb208-1", "4bb208", at(6, 2), at(6, 12), Kind::Arrival);
        thy.flight = Some("THY1".into());
        thy.ac_type = Some("A321".into());
        thy.reg = Some("TC-JRA".into());
        thy.arr_runway = Some("35R".into());
        thy.touchdown_ms = Some(at(6, 12) + 20_000);
        thy.asma_entry_ms = Some(at(5, 57) + 20_000);
        thy.asma_s = Some(900.0);
        let mut pgt = flight("4ba9c1-1", "4ba9c1", at(6, 9), at(6, 18), Kind::Departure);
        pgt.flight = Some("PGT2".into());
        pgt.ac_type = Some("B738".into());
        pgt.reg = Some("TC-CPA".into());
        pgt.liftoff_ms = Some(at(6, 8));
        pgt.time_low_confidence = true;
        let mut uae = flight("896180-1", "896180", at(6, 0), at(6, 30), Kind::Overflight);
        uae.flight = Some("UAE3".into());
        uae.ac_type = Some("A388".into());
        uae.reg = Some("A6-EOA".into());
        let mut td = event(Some("4bb208-1"), at(6, 12) + 20_000, EventKind::Touchdown);
        td.runway = Some("35R".into());
        td.lat = Some(41.2646);
        td.lon = Some(28.7277);
        let lo = event(Some("4ba9c1-1"), at(6, 8), EventKind::Liftoff);
        let mut cfg = event(None, at(6, 10), EventKind::ConfigChange);
        cfg.detail = Some("ARR 35R / DEP -".into());
        let bins = vec![Bin {
            bin_ms: at(6, 10),
            runway: "35R".into(),
            arrivals: 1,
            departures: 0,
            config: "ARR 35R / DEP -".into(),
        }];
        (twin(vec![thy, pgt, uae], vec![lo, cfg, td], bins), o)
    }

    fn write_day_json(dir: &std::path::Path, date: &str, asma: &[Option<i64>]) {
        let flights = asma
            .iter()
            .enumerate()
            .map(|(i, a)| FlightJ {
                id: format!("f{i}"),
                cs: None,
                ac_type: None,
                reg: None,
                kind: if a.is_some() { "arrival" } else { "departure" }.into(),
                arr_rwy: None,
                td: None,
                dep_rwy: None,
                lo: None,
                low: false,
                asma_s: *a,
                ga: 0,
                holds: 0,
                hold_s: 0,
                p: [0, 0],
            })
            .collect();
        let j = DayJson {
            v: 1,
            airport: "LTFM".into(),
            date: date.into(),
            day_start_ms: 0,
            flights,
            events: vec![],
            config: vec![],
            hist: HistJ {
                bin_s: 600,
                arr: vec![],
                dep: vec![],
            },
        };
        write_day(dir, "LTFM", date, &j, &[]).unwrap();
    }

    #[test]
    fn write_day_gzips_the_tracks() {
        use std::io::Read;
        let (tw, o) = two_flights();
        let (j, bin) = build("LTFM", day(), &tw, &o);
        let dir = tempfile::tempdir().unwrap();
        write_day(dir.path(), "LTFM", "2026-10-02", &j, &bin).unwrap();
        let day_dir = dir.path().join("LTFM/2026-10-02");
        assert!(!day_dir.join("tracks.bin").exists());
        let gz = std::fs::read(day_dir.join("tracks.bin.gz")).unwrap();
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(&gz[..])
            .read_to_end(&mut out)
            .unwrap();
        assert_eq!(out, bin);
    }

    #[test]
    fn index_lists_days_sorted_with_medians() {
        let dir = tempfile::tempdir().unwrap();
        write_day_json(dir.path(), "2026-10-03", &[Some(600)]);
        write_day_json(
            dir.path(),
            "2026-10-02",
            &[Some(1200), None, Some(600), Some(900)],
        );
        assert_eq!(write_index(dir.path(), "LTFM").unwrap(), 2);
        let idx: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("index.json")).unwrap()).unwrap();
        assert_eq!(idx["airport"], "LTFM");
        assert_eq!(idx["days"][0]["date"], "2026-10-02");
        assert_eq!(idx["days"][0]["arrivals"], 3);
        assert_eq!(idx["days"][0]["departures"], 1);
        assert_eq!(idx["days"][0]["asma_median_s"], 900);
        assert_eq!(idx["days"][1]["date"], "2026-10-03");
    }

    #[test]
    fn fixture_matches_committed_files() {
        let fixture = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../console/test/fixture"
        ));
        let (tw, o) = fixture_day();
        let (j, bin) = build("LTFM", day(), &tw, &o);
        let tmp = tempfile::tempdir().unwrap();
        write_day(tmp.path(), "LTFM", "2026-10-02", &j, &bin).unwrap();
        write_index(tmp.path(), "LTFM").unwrap();
        for rel in [
            "index.json",
            "LTFM/2026-10-02/day.json",
            "LTFM/2026-10-02/tracks.bin.gz",
        ] {
            let got = std::fs::read(tmp.path().join(rel)).unwrap();
            let path = fixture.join(rel);
            if std::env::var("UPDATE_FIXTURE").is_ok() {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, &got).unwrap();
            } else {
                let want = std::fs::read(&path)
                    .unwrap_or_else(|_| panic!("missing {rel}; run with UPDATE_FIXTURE=1"));
                assert!(got == want, "{rel} differs from the committed fixture");
            }
        }
    }
}
