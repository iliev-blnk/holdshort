//! Approach rows for anomaly detection: each arrival's path from the 40 nm
//! entry to touchdown, resampled in its runway's frame, plus a few metrics.

use std::collections::HashMap;

use holdshort_core::Obs;
use holdshort_core::airport::{Airport, RunwayEnd};

use crate::classify::Kind;
use crate::day::{DayTwin, Flight};
use crate::geo::{NM_M, heading_diff, to_runway};
use crate::height::{Weather, h_agl};

/// Resampled points per approach.
pub const M: usize = 32;
/// Values per resampled path: x, y, h for each point.
pub const PATH_N: usize = 3 * M;
pub const METRICS: [&str; 7] = [
    "extra_nm",
    "time_min",
    "turn_deg",
    "level_min",
    "high_10nm_ft",
    "gs_4nm_kt",
    "xtk_final_m",
];
/// Runways with enough approaches for a reference.
pub const REF_RUNWAYS: [&str; 2] = ["34L", "35R"];

/// One arrival's approach. Unscored rows carry NaN metrics and path.
#[derive(Debug, Clone, PartialEq)]
pub struct ApproachRow {
    pub flight_id: String,
    pub touchdown_ms: i64,
    pub runway: Option<String>,
    /// None when scored; else "lost_on_final", "no_entry", "runway" or "coverage".
    pub unscored: Option<&'static str>,
    pub label_ga: bool,
    pub label_hold: bool,
    pub metrics: [f32; 7],
    /// x0, y0, h0, x1, …: nm along and across the runway frame, thousands of ft.
    pub path: [f32; PATH_N],
}

/// Fewer points than this between entry and touchdown is not enough coverage.
const MIN_POINTS: usize = 20;
/// A longer gap between consecutive points is coverage loss.
const MAX_GAP_MS: i64 = 600_000;
/// Points closer than this to the last kept one are skipped when summing turns.
const TURN_STEP_NM: f64 = 0.3;
/// Climb or descent slower than this counts as level.
const LEVEL_FPM: f64 = 300.0;
const LEVEL_BELOW_FT: f64 = 10_000.0;
/// Height of a 3° glide path at 10 nm.
const GLIDE_10NM_FT: f64 = 3184.0;

/// A path point in the runway frame: nm along, nm across, ft above the threshold.
#[derive(Debug, Clone, Copy)]
struct P {
    t: i64,
    x: f64,
    y: f64,
    h: f64,
    gs: Option<f64>,
}

impl P {
    fn d(&self) -> f64 {
        self.x.hypot(self.y)
    }
    fn lerp(&self, o: &P, f: f64) -> P {
        P {
            t: self.t + ((o.t - self.t) as f64 * f).round() as i64,
            x: self.x + f * (o.x - self.x),
            y: self.y + f * (o.y - self.y),
            h: self.h + f * (o.h - self.h),
            gs: match (self.gs, o.gs) {
                (Some(a), Some(b)) => Some(a + f * (b - a)),
                (a, b) => b.or(a),
            },
        }
    }
}

fn is_arrival(f: &Flight) -> bool {
    matches!(f.kind, Kind::Arrival | Kind::Both)
}

/// One row per arrival with a touchdown.
pub fn approach_rows(ap: &Airport, twin: &DayTwin, obs: &[Obs], wx: &Weather) -> Vec<ApproachRow> {
    let mut by_hex: HashMap<&str, Vec<&Obs>> = HashMap::new();
    for o in obs.iter().filter(|o| !o.on_ground) {
        by_hex.entry(o.hex.as_str()).or_default().push(o);
    }
    for v in by_hex.values_mut() {
        v.sort_by_key(|o| o.ts_ms);
    }
    twin.flights
        .iter()
        .filter(|f| is_arrival(f))
        .filter_map(|f| {
            let td = f.touchdown_ms?;
            let pts = by_hex.get(f.hex.as_str()).map_or(&[][..], |v| v.as_slice());
            Some(row(ap, f, td, pts, wx))
        })
        .collect()
}

fn row(ap: &Airport, f: &Flight, td: i64, pts: &[&Obs], wx: &Weather) -> ApproachRow {
    let mut r = ApproachRow {
        flight_id: f.id.clone(),
        touchdown_ms: td,
        runway: f.arr_runway.clone(),
        unscored: None,
        label_ga: f.go_arounds > 0,
        label_hold: f.holds > 0,
        metrics: [f32::NAN; 7],
        path: [f32::NAN; PATH_N],
    };
    let rwy = f
        .arr_runway
        .as_deref()
        .filter(|n| REF_RUNWAYS.contains(n))
        .and_then(|n| ap.runways.iter().find(|r| r.name == n));
    r.unscored = if f.time_low_confidence {
        Some("lost_on_final")
    } else if f.asma_entry_ms.is_none() || f.asma_s.is_none() {
        Some("no_entry")
    } else if rwy.is_none() {
        Some("runway")
    } else {
        None
    };
    let (Some(rwy), None, Some(entry), Some(asma_s)) = (rwy, r.unscored, f.asma_entry_ms, f.asma_s)
    else {
        return r;
    };
    let Some(poly) = polyline(rwy, entry, td, pts, wx) else {
        r.unscored = Some("coverage");
        return r;
    };
    let Some(path) = resample(&poly) else {
        r.unscored = Some("coverage");
        return r;
    };
    r.path = path;
    r.metrics = metrics(&poly, asma_s);
    r
}

/// Entry (interpolated), the airborne points strictly inside, and the
/// threshold at touchdown; None when coverage is not good enough.
fn polyline(rwy: &RunwayEnd, entry: i64, td: i64, pts: &[&Obs], wx: &Weather) -> Option<Vec<P>> {
    let to_p = |o: &Obs| -> Option<P> {
        let h = h_agl(o, wx.at(o.ts_ms), rwy.elevation_ft)?;
        let l = to_runway(rwy, o.lat, o.lon);
        Some(P {
            t: o.ts_ms,
            x: l.along_m / NM_M,
            y: l.cross_m / NM_M,
            h,
            gs: o.gs_kt.map(f64::from),
        })
    };
    let ps: Vec<P> = pts.iter().filter_map(|o| to_p(o)).collect();
    let before = ps.iter().rev().find(|p| p.t <= entry)?;
    let inside: Vec<P> = ps
        .iter()
        .filter(|p| p.t > entry && p.t < td)
        .copied()
        .collect();
    if inside.len() < MIN_POINTS {
        return None;
    }
    let first = inside[0];
    let f = if first.t == before.t {
        0.0
    } else {
        (entry - before.t) as f64 / (first.t - before.t) as f64
    };
    let mut poly = Vec::with_capacity(inside.len() + 2);
    poly.push(before.lerp(&first, f));
    poly.extend(inside.iter().copied());
    let last_gs = inside.last().and_then(|p| p.gs);
    poly.push(P {
        t: td,
        x: 0.0,
        y: 0.0,
        h: 0.0,
        gs: last_gs,
    });
    if poly.windows(2).any(|w| w[1].t - w[0].t > MAX_GAP_MS) {
        return None;
    }
    Some(poly)
}

/// Cumulative 2-D length at each point, nm.
fn cumulative(poly: &[P]) -> Vec<f64> {
    let mut s = vec![0.0];
    for w in poly.windows(2) {
        s.push(s.last().unwrap() + (w[1].x - w[0].x).hypot(w[1].y - w[0].y));
    }
    s
}

/// M points equally spaced in along-track distance.
fn resample(poly: &[P]) -> Option<[f32; PATH_N]> {
    let s = cumulative(poly);
    let total = *s.last()?;
    if total <= 0.0 {
        return None;
    }
    let mut out = [0f32; PATH_N];
    let mut j = 0;
    for i in 0..M {
        let u = total * i as f64 / (M - 1) as f64;
        while j + 2 < poly.len() && s[j + 1] < u {
            j += 1;
        }
        let seg = s[j + 1] - s[j];
        let f = if seg > 0.0 {
            ((u - s[j]) / seg).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let p = poly[j].lerp(&poly[j + 1], f);
        out[3 * i] = p.x as f32;
        out[3 * i + 1] = p.y as f32;
        out[3 * i + 2] = (p.h / 1000.0) as f32;
    }
    Some(out)
}

/// The point where the path last crosses inward through `r_nm` from the threshold.
fn last_crossing(poly: &[P], r_nm: f64) -> Option<(usize, P)> {
    poly.windows(2).enumerate().rev().find_map(|(i, w)| {
        let (d0, d1) = (w[0].d(), w[1].d());
        (d0 > r_nm && d1 <= r_nm).then(|| (i, w[0].lerp(&w[1], (d0 - r_nm) / (d0 - d1))))
    })
}

fn metrics(poly: &[P], asma_s: f64) -> [f32; 7] {
    let length = *cumulative(poly).last().unwrap_or(&0.0);
    let direct = poly[0].d();

    let mut turn = 0.0;
    let mut kept = poly[0];
    let mut last_brg: Option<f64> = None;
    for p in &poly[1..] {
        if (p.x - kept.x).hypot(p.y - kept.y) < TURN_STEP_NM {
            continue;
        }
        let brg = (p.y - kept.y).atan2(p.x - kept.x).to_degrees();
        if let Some(b) = last_brg {
            turn += heading_diff(brg, b);
        }
        last_brg = Some(brg);
        kept = *p;
    }

    let level_ms: i64 = poly
        .windows(2)
        .filter(|w| {
            let dt = w[1].t - w[0].t;
            dt > 0
                && w[0].h < LEVEL_BELOW_FT
                && w[1].h < LEVEL_BELOW_FT
                && ((w[1].h - w[0].h).abs() / dt as f64 * 60_000.0) < LEVEL_FPM
        })
        .map(|w| w[1].t - w[0].t)
        .sum();

    let high_10 = last_crossing(poly, 10.0).map_or(f64::NAN, |(_, p)| p.h - GLIDE_10NM_FT);
    let gs_4 = last_crossing(poly, 4.0)
        .and_then(|(_, p)| p.gs)
        .unwrap_or(f64::NAN);
    let xtk = last_crossing(poly, 8.0).map_or(f64::NAN, |(i, _)| {
        poly[i + 1..]
            .iter()
            .filter(|p| (-8.0..=-2.0).contains(&p.x))
            .map(|p| p.y.abs() * NM_M)
            .fold(f64::NAN, f64::max)
    });

    [
        (length - direct) as f32,
        (asma_s / 60.0) as f32,
        turn as f32,
        (level_ms as f64 / 60_000.0) as f32,
        high_10 as f32,
        gs_4 as f32,
        xtk as f32,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::from_runway;
    use crate::testkit::{point, with_hex};
    use holdshort_core::airport::LTFM;

    const T0: i64 = 1_790_000_000_000;
    const STEP: i64 = 10_000;
    /// 3° glide path, ft per nm.
    const GLIDE: f64 = 318.4;

    fn rwy(name: &str) -> &'static RunwayEnd {
        LTFM.runways.iter().find(|r| r.name == name).unwrap()
    }

    /// A track through runway-frame waypoints (nm along, nm across, ft above
    /// the threshold), 0.5 nm every 10 s (180 kt).
    fn track(r: &RunwayEnd, wps: &[(f64, f64, f64)], t0: i64) -> Vec<Obs> {
        let mut out = Vec::new();
        let mut t = t0;
        for w in wps.windows(2) {
            let ((x0, y0, h0), (x1, y1, h1)) = (w[0], w[1]);
            let len = (x1 - x0).hypot(y1 - y0);
            let n = (len / 0.5).ceil().max(1.0) as usize;
            let trk = r.heading_deg + (y1 - y0).atan2(x1 - x0).to_degrees();
            for i in 0..n {
                let f = i as f64 / n as f64;
                let (x, y, h) = (x0 + f * (x1 - x0), y0 + f * (y1 - y0), h0 + f * (h1 - h0));
                let (lat, lon) = from_runway(r, x * NM_M, y * NM_M);
                out.push(point(t, lat, lon, r.elevation_ft + h, 180.0, trk, 0.0));
                t += STEP;
            }
        }
        let &(x, y, h) = wps.last().unwrap();
        let (lat, lon) = from_runway(r, x * NM_M, y * NM_M);
        out.push(point(
            t,
            lat,
            lon,
            r.elevation_ft + h,
            180.0,
            r.heading_deg,
            0.0,
        ));
        out
    }

    fn straight_in(r: &RunwayEnd, t0: i64) -> Vec<Obs> {
        track(
            r,
            &[(-42.0, 0.0, 42.0 * GLIDE), (-0.6, 0.0, 0.6 * GLIDE)],
            t0,
        )
    }

    /// An arrival entering 5 s after the 5th point, landing 12 s after the last.
    fn arrival(id: &str, hex: &str, pts: &[Obs], rwy: &str) -> Flight {
        let mut f = Flight::empty(id, pts[0].ts_ms);
        f.hex = hex.into();
        f.kind = Kind::Arrival;
        f.last_ms = pts.last().unwrap().ts_ms;
        let entry = pts[4].ts_ms + 5_000;
        let td = f.last_ms + 12_000;
        f.asma_entry_ms = Some(entry);
        f.touchdown_ms = Some(td);
        f.asma_s = Some((td - entry) as f64 / 1000.0);
        f.arr_runway = Some(rwy.into());
        f
    }

    fn twin(flights: Vec<Flight>) -> DayTwin {
        DayTwin {
            flights,
            events: vec![],
            bins: vec![],
        }
    }

    fn rows(flights: Vec<Flight>, obs: &[Obs]) -> Vec<ApproachRow> {
        approach_rows(&LTFM, &twin(flights), obs, &Weather::new(&[]))
    }

    fn one(flights: Vec<Flight>, obs: &[Obs]) -> ApproachRow {
        let mut r = rows(flights, obs);
        assert_eq!(r.len(), 1);
        r.remove(0)
    }

    fn m(row: &ApproachRow, name: &str) -> f32 {
        row.metrics[METRICS.iter().position(|n| *n == name).unwrap()]
    }

    #[test]
    fn straight_in_path_is_on_centreline() {
        let pts = with_hex(straight_in(rwy("35R"), T0), "a1");
        let r = one(vec![arrival("f1", "a1", &pts, "35R")], &pts);
        assert_eq!(r.unscored, None);
        for i in 0..M {
            assert!(
                r.path[3 * i + 1].abs() <= 0.01,
                "y{i} = {}",
                r.path[3 * i + 1]
            );
        }
        // Entry is half way between the 5th and 6th points: 42 − 2.25 nm out.
        assert!((r.path[0] + 39.75).abs() < 0.05, "x0 = {}", r.path[0]);
        assert!((r.path[2] - 39.75 * GLIDE as f32 / 1000.0).abs() < 0.05);
        assert_eq!(&r.path[PATH_N - 3..], &[0.0, 0.0, 0.0]);
        assert!(m(&r, "extra_nm").abs() < 0.2, "{}", m(&r, "extra_nm"));
        assert!(m(&r, "turn_deg") < 5.0);
        assert!(
            m(&r, "high_10nm_ft").abs() < 100.0,
            "{}",
            m(&r, "high_10nm_ft")
        );
        assert!(m(&r, "xtk_final_m") < 5.0);
        assert!((m(&r, "gs_4nm_kt") - 180.0).abs() < 0.1);
        assert!(
            (m(&r, "time_min") - (r.touchdown_ms - T0 - 45_000) as f32 / 60_000.0).abs() < 1e-3
        );
        // A 3° descent is never level.
        assert_eq!(m(&r, "level_min"), 0.0);
        assert!(!r.label_ga && !r.label_hold);
    }

    #[test]
    fn resampled_points_are_equally_spaced() {
        let pts = with_hex(straight_in(rwy("34L"), T0), "a1");
        let r = one(vec![arrival("f1", "a1", &pts, "34L")], &pts);
        let step = -r.path[0] / (M - 1) as f32;
        for i in 1..M {
            let d = r.path[3 * i] - r.path[3 * (i - 1)];
            assert!((d - step).abs() < 1e-3, "step {i}: {d} vs {step}");
        }
    }

    #[test]
    fn orbit_adds_distance_and_turn() {
        let r35 = rwy("35R");
        let h = 15.0 * GLIDE;
        let mut wps = vec![(-42.0, 0.0, 42.0 * GLIDE), (-15.0, 0.0, h)];
        // A right-hand 360° of radius 1.5 nm, back to where it began.
        for k in 1..=24 {
            let th = std::f64::consts::TAU * k as f64 / 24.0;
            wps.push((-15.0 + 1.5 * th.sin(), 1.5 - 1.5 * th.cos(), h));
        }
        wps.push((-0.6, 0.0, 0.6 * GLIDE));
        let pts = with_hex(track(r35, &wps, T0), "a1");
        let r = one(vec![arrival("f1", "a1", &pts, "35R")], &pts);
        let extra = m(&r, "extra_nm") as f64;
        assert!(
            (extra - std::f64::consts::TAU * 1.5).abs() < 1.0,
            "extra {extra}"
        );
        let turn = m(&r, "turn_deg");
        assert!((300.0..=420.0).contains(&turn), "turn {turn}");
        assert!(
            m(&r, "level_min") > 0.5,
            "orbit is level: {}",
            m(&r, "level_min")
        );
    }

    #[test]
    fn go_around_uses_last_crossing() {
        // First approach 2,000 ft high at 10 nm, missed at 1.5 nm, then a
        // left-hand circuit and a normal 3° final from 15 nm.
        let wps = [
            (-42.0, 0.0, 12_000.0),
            (-10.0, 0.0, 10.0 * GLIDE + 2000.0),
            (-1.5, 0.0, 500.0),
            (3.0, 0.0, 3000.0),
            (3.0, -4.0, 3000.0),
            (-15.0, -4.0, 15.0 * GLIDE),
            (-15.0, 0.0, 15.0 * GLIDE),
            (-0.6, 0.0, 0.6 * GLIDE),
        ];
        let pts = with_hex(track(rwy("35R"), &wps, T0), "a1");
        let mut f = arrival("f1", "a1", &pts, "35R");
        f.go_arounds = 1;
        let r = one(vec![f], &pts);
        assert!(
            m(&r, "high_10nm_ft").abs() < 150.0,
            "{}",
            m(&r, "high_10nm_ft")
        );
        assert!(m(&r, "xtk_final_m") < 5.0, "{}", m(&r, "xtk_final_m"));
        assert!(r.label_ga);
    }

    #[test]
    fn high_approach_is_measured_at_10nm() {
        let wps = [
            (-42.0, 0.0, 12_000.0),
            (-10.0, 0.0, 10.0 * GLIDE + 2000.0),
            (-0.6, 0.0, 0.6 * GLIDE),
        ];
        let pts = with_hex(track(rwy("35R"), &wps, T0), "a1");
        let r = one(vec![arrival("f1", "a1", &pts, "35R")], &pts);
        assert!(
            (m(&r, "high_10nm_ft") - 2000.0).abs() < 60.0,
            "{}",
            m(&r, "high_10nm_ft")
        );
    }

    #[test]
    fn offset_final_shows_cross_track() {
        let wps = [
            (-42.0, 0.3, 42.0 * GLIDE),
            (-8.5, 0.3, 8.5 * GLIDE),
            (-2.0, 0.3, 2.0 * GLIDE),
            (-0.6, 0.0, 0.6 * GLIDE),
        ];
        let pts = with_hex(track(rwy("35R"), &wps, T0), "a1");
        let r = one(vec![arrival("f1", "a1", &pts, "35R")], &pts);
        let x = m(&r, "xtk_final_m");
        assert!((x - 0.3 * 1852.0).abs() < 5.0, "{x}");
    }

    #[test]
    fn coverage_gate() {
        let pts = with_hex(straight_in(rwy("35R"), T0), "a1");
        let f = arrival("f1", "a1", &pts, "35R");
        let (entry, td) = (f.asma_entry_ms.unwrap(), f.touchdown_ms.unwrap());
        let inside: Vec<usize> = (0..pts.len())
            .filter(|&i| pts[i].ts_ms > entry && pts[i].ts_ms < td)
            .collect();
        // The bracketing point before entry plus 19 points inside, 40 s apart.
        let mut few: Vec<Obs> = pts[..5].to_vec();
        few.extend(inside.iter().step_by(4).take(19).map(|&i| pts[i].clone()));
        assert_eq!(
            few.iter()
                .filter(|o| o.ts_ms > entry && o.ts_ms < td)
                .count(),
            19
        );
        let r = one(vec![f.clone()], &few);
        assert_eq!(r.unscored, Some("coverage"), "19 points");
        assert!(r.metrics[0].is_nan() && r.path[0].is_nan());

        // A gap of exactly 600 s is allowed, 610 s is not.
        let gap = |secs: i64| -> Vec<Obs> {
            let a = pts[20].ts_ms;
            pts.iter()
                .filter(|o| o.ts_ms <= a || o.ts_ms >= a + secs * 1000)
                .cloned()
                .collect()
        };
        assert_eq!(one(vec![f.clone()], &gap(600)).unscored, None);
        assert_eq!(one(vec![f], &gap(610)).unscored, Some("coverage"));
    }

    #[test]
    fn two_flights_same_hex_do_not_mix() {
        let r = rwy("35R");
        let a = with_hex(straight_in(r, T0), "a1");
        let b = with_hex(straight_in(r, T0 + 3 * 3_600_000), "a1");
        let fa = arrival("fa", "a1", &a, "35R");
        let fb = arrival("fb", "a1", &b, "35R");
        let mut obs = a.clone();
        obs.extend(b.iter().cloned());
        let rs = rows(vec![fa, fb], &obs);
        assert_eq!(rs.len(), 2);
        for row in &rs {
            assert_eq!(row.unscored, None, "{}", row.flight_id);
            assert!((row.path[0] + 39.75).abs() < 0.05);
            assert!(row.metrics[0].abs() < 0.2);
        }
    }

    #[test]
    fn entry_before_first_point_is_coverage() {
        let pts = with_hex(straight_in(rwy("35R"), T0), "a1");
        let mut f = arrival("f1", "a1", &pts, "35R");
        f.asma_entry_ms = Some(T0 - 1_000);
        assert_eq!(one(vec![f], &pts).unscored, Some("coverage"));
    }

    #[test]
    fn unscored_reasons() {
        let pts = with_hex(straight_in(rwy("35R"), T0), "a1");
        let base = arrival("f1", "a1", &pts, "35R");

        let mut low = base.clone();
        low.time_low_confidence = true;
        low.asma_entry_ms = None;
        assert_eq!(one(vec![low], &pts).unscored, Some("lost_on_final"));

        let mut no_entry = base.clone();
        no_entry.asma_entry_ms = None;
        no_entry.asma_s = None;
        assert_eq!(one(vec![no_entry], &pts).unscored, Some("no_entry"));

        let mut r36 = base.clone();
        r36.arr_runway = Some("36".into());
        r36.holds = 2;
        let row = one(vec![r36], &pts);
        assert_eq!(row.unscored, Some("runway"));
        assert!(row.label_hold && !row.label_ga);

        let mut no_rwy = base.clone();
        no_rwy.arr_runway = None;
        assert_eq!(one(vec![no_rwy], &pts).unscored, Some("runway"));
    }

    #[test]
    fn only_arrivals_with_touchdown_get_rows() {
        let pts = with_hex(straight_in(rwy("35R"), T0), "a1");
        let mut dep = arrival("d1", "a1", &pts, "35R");
        dep.kind = Kind::Departure;
        let mut no_td = arrival("f2", "a1", &pts, "35R");
        no_td.touchdown_ms = None;
        let mut both = arrival("b1", "a1", &pts, "35R");
        both.kind = Kind::Both;
        let rs = rows(vec![dep, no_td, both], &pts);
        assert_eq!(
            rs.iter().map(|r| r.flight_id.as_str()).collect::<Vec<_>>(),
            ["b1"]
        );
    }
}
