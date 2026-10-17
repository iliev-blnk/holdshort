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
use holdshort_models::EtaModel;
use holdshort_models::approach::{ApproachModel, ApproachScore};
use holdshort_twin::approach::{ApproachRow, approach_rows};
use holdshort_twin::features::eta_rows;
use holdshort_twin::height::Weather;

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
    /// Landing time predicted at the 40 nm ring, seconds after entry (with `--models`).
    pub eta_s: Option<i64>,
    /// The approach anomaly score (arrivals, with `--models`).
    pub ap: Option<ApJ>,
    /// The nearest normal approach as `[lon, lat]` points (flagged approaches only).
    pub ap_ref: Option<Vec<[f64; 2]>>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct ApJ {
    /// Percentile among the validation week's approaches to the same runway, 1 decimal.
    pub pct: Option<f64>,
    pub flag: bool,
    pub reasons: Vec<ReasonJ>,
    /// Why the approach has no score ("lost_on_final", "no_entry", "runway", "coverage").
    pub unscored: Option<String>,
}

/// A metric far from normal: its value and the typical (median) value, 1 decimal.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct ReasonJ {
    pub m: String,
    pub v: f32,
    pub typ: f32,
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
            eta_s: None,
            ap: None,
            ap_ref: None,
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

/// Set each flight's predicted landing time from `eta` (flight id → seconds).
pub fn attach_eta(json: &mut DayJson, eta: &HashMap<String, f32>) {
    for f in &mut json.flights {
        f.eta_s = eta.get(&f.id).map(|s| s.round() as i64);
    }
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// Set each arrival's approach score from `rows` and their `scores` (same order), the
/// nearest normal approach of flagged ones from `refs` (flight id → points), and add an
/// `unusual_approach` event at each flagged touchdown.
pub fn attach_approach(
    json: &mut DayJson,
    rows: &[ApproachRow],
    scores: &[Option<ApproachScore>],
    refs: &HashMap<String, Vec<[f64; 2]>>,
) {
    let index: HashMap<String, usize> = json
        .flights
        .iter()
        .enumerate()
        .map(|(i, f)| (f.id.clone(), i))
        .collect();
    let mut events = Vec::new();
    for (row, score) in rows.iter().zip(scores) {
        let Some(&i) = index.get(&row.flight_id) else {
            continue;
        };
        let ap = match score {
            Some(s) => ApJ {
                pct: Some(round1(s.pct)),
                flag: s.flag,
                reasons: s
                    .reasons
                    .iter()
                    .map(|r| ReasonJ {
                        m: r.metric.into(),
                        v: round1(r.value as f64) as f32,
                        typ: round1(r.typical as f64) as f32,
                    })
                    .collect(),
                unscored: None,
            },
            None => ApJ {
                pct: None,
                flag: false,
                reasons: vec![],
                unscored: Some(row.unscored.unwrap_or("runway").into()),
            },
        };
        if ap.flag {
            json.flights[i].ap_ref = refs.get(&row.flight_id).map(|pts| {
                pts.iter()
                    .map(|p| p.map(|v| (v * 1e5).round() / 1e5))
                    .collect()
            });
            events.push(EventJ {
                t: row.touchdown_ms - json.day_start_ms,
                kind: "unusual_approach".into(),
                f: Some(i as u32),
                rwy: row.runway.clone(),
                lat: None,
                lon: None,
                detail: None,
            });
        }
        json.flights[i].ap = Some(ap);
    }
    json.events.extend(events);
    json.events.sort_by_key(|e| e.t);
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
    models: Option<&Path>,
) -> anyhow::Result<String> {
    let (twin, obs, metars) = compute_day(store, ap, day)?;
    let (mut json, tracks) = build(ap.icao, day, &twin, &obs);
    if let Some(dir) = models {
        let rows = eta_rows(ap, &twin, &obs, &Weather::new(&metars));
        let x: Vec<_> = rows.iter().map(|r| r.x).collect();
        let pred = EtaModel::load(dir)?.predict(&x)?;
        let eta = rows.into_iter().map(|r| r.flight_id).zip(pred).collect();
        attach_eta(&mut json, &eta);

        let model = ApproachModel::load(dir)?;
        let rows = approach_rows(ap, &twin, &obs, &Weather::new(&metars));
        let scores: Vec<_> = rows.iter().map(|r| model.score(r)).collect();
        let mut refs = HashMap::new();
        for (r, s) in rows.iter().zip(&scores) {
            let (Some(s), Some(name)) = (s, r.runway.as_deref()) else {
                continue;
            };
            if let (true, Some(end)) = (s.flag, ap.runways.iter().find(|e| e.name == name)) {
                refs.insert(
                    r.flight_id.clone(),
                    model.nearest_path(end, name, s.nearest),
                );
            }
        }
        attach_approach(&mut json, &rows, &scores, &refs);
    }
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
    use holdshort_models::approach::Reason;
    use holdshort_twin::approach::PATH_N;
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
    fn export_without_models_has_no_eta() {
        let (tw, o) = two_flights();
        let (j, _) = build("LTFM", day(), &tw, &o);
        assert!(j.flights.iter().all(|f| f.eta_s.is_none()));
        assert!(
            serde_json::to_string(&j)
                .unwrap()
                .contains("\"eta_s\":null")
        );
    }

    #[test]
    fn attach_eta_sets_predictions_by_flight_id() {
        let (tw, o) = two_flights();
        let (mut j, _) = build("LTFM", day(), &tw, &o);
        let eta = std::collections::HashMap::from([
            ("bbb-1".to_string(), 912.4f32),
            ("nope".to_string(), 1.0),
        ]);
        attach_eta(&mut j, &eta);
        assert_eq!(j.flights[0].eta_s, None);
        assert_eq!(j.flights[1].eta_s, Some(912));
    }

    #[test]
    fn export_without_models_has_no_ap() {
        let (tw, o) = two_flights();
        let (j, _) = build("LTFM", day(), &tw, &o);
        assert!(
            j.flights
                .iter()
                .all(|f| f.ap.is_none() && f.ap_ref.is_none())
        );
        let s = serde_json::to_string(&j).unwrap();
        assert!(s.contains("\"ap\":null") && s.contains("\"ap_ref\":null"));
    }

    fn approach_row(id: &str, td: i64, unscored: Option<&'static str>) -> ApproachRow {
        ApproachRow {
            flight_id: id.into(),
            touchdown_ms: td,
            runway: Some("35R".into()),
            unscored,
            label_ga: false,
            label_hold: false,
            metrics: [0.0; 7],
            path: [0.0; PATH_N],
        }
    }

    fn approach_score(pct: f64, flag: bool) -> ApproachScore {
        ApproachScore {
            score: 1.0,
            pct,
            flag,
            reasons: if flag {
                vec![Reason {
                    metric: "extra_nm",
                    value: 18.43,
                    typical: 4.06,
                }]
            } else {
                vec![]
            },
            nearest: 0,
        }
    }

    #[test]
    fn attach_approach_flags_and_events() {
        let at = |m: i64| START + 6 * 60 * MIN + m * MIN;
        let mut fl = Vec::new();
        for (i, id) in ["a-1", "b-1", "c-1"].iter().enumerate() {
            let mut f = flight(id, "h", at(0), at(30), Kind::Arrival);
            f.touchdown_ms = Some(at(10 * i as i64 + 10));
            fl.push(f);
        }
        let evs = vec![
            event(Some("a-1"), at(10), EventKind::Touchdown),
            event(Some("b-1"), at(20), EventKind::Touchdown),
            event(Some("c-1"), at(30), EventKind::Touchdown),
        ];
        let (mut j, _) = build("LTFM", day(), &twin(fl, evs, vec![]), &[]);
        let rows = vec![
            approach_row("a-1", at(10), None),
            approach_row("b-1", at(20), None),
            approach_row("c-1", at(30), Some("lost_on_final")),
        ];
        let scores = vec![
            Some(approach_score(99.46, true)),
            Some(approach_score(54.04, false)),
            None,
        ];
        let refs = HashMap::from([("a-1".to_string(), vec![[28.123456789, 41.0]; 32])]);
        attach_approach(&mut j, &rows, &scores, &refs);

        let a = j.flights[0].ap.as_ref().unwrap();
        assert_eq!(a.pct, Some(99.5));
        assert!(a.flag);
        assert_eq!(
            a.reasons,
            [ReasonJ {
                m: "extra_nm".into(),
                v: 18.4,
                typ: 4.1
            }]
        );
        let r = j.flights[0].ap_ref.as_ref().unwrap();
        assert_eq!(r.len(), 32);
        assert_eq!(r[0], [28.12346, 41.0]);

        let b = j.flights[1].ap.as_ref().unwrap();
        assert_eq!(
            (b.pct, b.flag, b.unscored.as_deref()),
            (Some(54.0), false, None)
        );
        assert!(j.flights[1].ap_ref.is_none());

        let c = j.flights[2].ap.as_ref().unwrap();
        assert_eq!(
            (c.pct, c.unscored.as_deref()),
            (None, Some("lost_on_final"))
        );

        let unusual: Vec<_> = j
            .events
            .iter()
            .filter(|e| e.kind == "unusual_approach")
            .collect();
        assert_eq!(unusual.len(), 1);
        assert_eq!(unusual[0].f, Some(0));
        assert_eq!(unusual[0].t, at(10) - START);
        assert_eq!(unusual[0].rwy.as_deref(), Some("35R"));
        assert!(j.events.windows(2).all(|w| w[0].t <= w[1].t));
        // Right after its touchdown.
        assert_eq!(j.events[1].kind, "unusual_approach");
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
                eta_s: None,
                ap: None,
                ap_ref: None,
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
        let (mut j, bin) = build("LTFM", day(), &tw, &o);
        // THY1 is a flagged approach, with a straight nearest normal approach from 10 nm.
        let thy = &tw.flights[0];
        let rows = [approach_row(&thy.id, thy.touchdown_ms.unwrap(), None)];
        let r = holdshort_core::airport::LTFM
            .runways
            .iter()
            .find(|r| r.name == "35R")
            .unwrap();
        let path = (0..32)
            .map(|i| {
                let (lat, lon) = holdshort_twin::geo::from_runway(
                    r,
                    -1852.0 * (10.0 - i as f64 * 10.0 / 31.0),
                    0.0,
                );
                [lon, lat]
            })
            .collect();
        attach_approach(
            &mut j,
            &rows,
            &[Some(ApproachScore {
                score: 9.0,
                pct: 99.7,
                flag: true,
                reasons: vec![Reason {
                    metric: "extra_nm",
                    value: 18.0,
                    typical: 4.0,
                }],
                nearest: 0,
            })],
            &HashMap::from([(thy.id.clone(), path)]),
        );
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
