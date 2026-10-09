//! Arrival, departure, overflight or other; and which runway.

use holdshort_core::Obs;
use holdshort_core::airport::{Airport, RunwayEnd};
use serde::{Deserialize, Serialize};

use crate::flights::Track;
use crate::geo::{NM_M, heading_diff, to_runway};
use crate::height::{Weather, h_agl};

const ALIGN_DEG: f64 = 15.0;
const MAX_CROSS_M: f64 = 1_500.0;
const LOW_FT: f64 = 2_000.0;
const END_FT: f64 = 1_500.0;
const MIN_POINTS: usize = 3;
const OVERFLIGHT_FT: f64 = 5_000.0;
const HIGH_DEP_NM: f64 = 25.0;
const HIGH_DEP_FT: f64 = 20_000.0;
const MIN_CLIMB_FPM: f64 = 300.0;
const MIN_GRADIENT_FT_PER_NM: f64 = 200.0;
/// A runway margin below this means the runway choice is ambiguous.
pub const AMBIGUOUS_M: f64 = 60.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Arrival,
    Departure,
    Both,
    Overflight,
    Other,
}

/// The runway a flight used, how clearly, and which points showed it.
#[derive(Debug, Clone)]
pub struct Fit {
    pub runway: &'static RunwayEnd,
    /// Mean cross-track distance to the second-best runway minus the best
    /// (infinite when only one runway matched).
    pub margin_m: f64,
    /// Indices into `track.points` of the points that matched.
    pub idx: Vec<usize>,
}

fn height(p: &Obs, wx: &Weather, r: &RunwayEnd) -> Option<f64> {
    h_agl(p, wx.at(p.ts_ms), r.elevation_ft)
}

/// Whether point `p` is lined up with `r` inside the along-track window.
pub(crate) fn aligned(p: &Obs, r: &RunwayEnd, along: (f64, f64)) -> Option<f64> {
    let trk = p.track_deg? as f64;
    if heading_diff(trk, r.heading_deg) > ALIGN_DEG {
        return None;
    }
    let l = to_runway(r, p.lat, p.lon);
    (l.along_m >= along.0 && l.along_m <= along.1 && l.cross_m.abs() <= MAX_CROSS_M)
        .then_some(l.cross_m.abs())
}

/// Best runway among candidates `(runway, mean |cross|, idx)`.
fn best(mut cands: Vec<(&'static RunwayEnd, f64, Vec<usize>)>) -> Option<Fit> {
    cands.sort_by(|a, b| a.1.total_cmp(&b.1));
    let margin_m = match cands.get(1) {
        Some(second) => second.1 - cands[0].1,
        None => f64::INFINITY,
    };
    let (runway, _, idx) = cands.into_iter().next()?;
    Some(Fit {
        runway,
        margin_m,
        idx,
    })
}

/// Score a run of points (given as indices, nearest the runway first) against
/// every runway end.
fn fit(
    t: &Track,
    ap: &'static Airport,
    wx: &Weather,
    order: &[usize],
    along: (f64, f64),
) -> Option<Fit> {
    let mut cands = Vec::new();
    for r in ap.runways {
        // The run of points near the runway: from the end (or start) of the
        // flight while lower than 2,000 ft above this runway.
        let low: Vec<usize> = order
            .iter()
            .copied()
            .take_while(|&i| height(&t.points[i], wx, r).is_some_and(|h| h < LOW_FT))
            .collect();
        let edge_ok = low
            .first()
            .and_then(|&i| height(&t.points[i], wx, r))
            .is_some_and(|h| h < END_FT);
        if !edge_ok {
            continue;
        }
        let mut idx = Vec::new();
        let mut sum = 0.0;
        for i in low {
            if let Some(c) = aligned(&t.points[i], r, along) {
                idx.push(i);
                sum += c;
            }
        }
        if idx.len() >= MIN_POINTS {
            idx.sort();
            cands.push((r, sum / idx.len() as f64, idx));
        }
    }
    best(cands)
}

/// The runway this flight landed on, judged from its last points.
pub fn arrival_fit(t: &Track, ap: &'static Airport, wx: &Weather) -> Option<Fit> {
    let order: Vec<usize> = (0..t.points.len()).rev().collect();
    fit(t, ap, wx, &order, (-6.0 * NM_M, 1_000.0))
}

/// The runway this flight took off from, judged from its first points.
pub fn departure_fit(t: &Track, ap: &'static Airport, wx: &Weather) -> Option<Fit> {
    let order: Vec<usize> = (0..t.points.len()).collect();
    fit(t, ap, wx, &order, (0.0, 8.0 * NM_M))
}

/// Every point has a height, and all are at least 5,000 ft above the airport.
pub fn is_overflight(t: &Track, ap: &Airport, wx: &Weather) -> bool {
    let elev = ap
        .runways
        .iter()
        .map(|r| r.elevation_ft)
        .fold(f64::INFINITY, f64::min);
    !t.points.is_empty()
        && t.points
            .iter()
            .all(|p| h_agl(p, wx.at(p.ts_ms), elev).is_some_and(|h| h >= OVERFLIGHT_FT))
}

/// A departure first seen climbing away from the airport above the low
/// coverage: first point within 25 nm and below 20,000 ft, climbing at least
/// 300 ft/min, and further from the airport a minute later. Receivers south
/// of LTFM often only see northbound departures from ~4,000–16,000 ft.
pub fn departs_seen_high(t: &Track, ap: &Airport, wx: &Weather) -> bool {
    let Some(first) = t.points.first() else {
        return false;
    };
    let elev = ap
        .runways
        .iter()
        .map(|r| r.elevation_ft)
        .fold(f64::INFINITY, f64::min);
    let d = |p: &Obs| holdshort_core::area::distance_nm(ap.lat, ap.lon, p.lat, p.lon);
    let Some(h0) = h_agl(first, wx.at(first.ts_ms), elev) else {
        return false;
    };
    // An airliner climbs far steeper than 200 ft/nm; anything lower this far
    // out left another airfield (e.g. Hezarfen, 9 nm SSW).
    let min_h = (d(first) - 1.0).max(0.0) * MIN_GRADIENT_FT_PER_NM;
    if d(first) > HIGH_DEP_NM || h0 >= HIGH_DEP_FT || h0 < min_h {
        return false;
    }
    let Some(later) = t.points.iter().find(|p| p.ts_ms - first.ts_ms >= 60_000) else {
        return false;
    };
    climb_fpm(first, later, wx, elev).is_some_and(|c| c >= MIN_CLIMB_FPM) && d(later) > d(first)
}

/// Climb rate at the first point: its reported rate, else measured to `later`.
pub fn climb_fpm(first: &Obs, later: &Obs, wx: &Weather, elev: f64) -> Option<f64> {
    if let Some(v) = first.vrate_fpm {
        return Some(v as f64);
    }
    let h0 = h_agl(first, wx.at(first.ts_ms), elev)?;
    let h1 = h_agl(later, wx.at(later.ts_ms), elev)?;
    let mins = (later.ts_ms - first.ts_ms) as f64 / 60_000.0;
    (mins > 0.0).then(|| (h1 - h0) / mins)
}

pub fn kind(arr: bool, dep: bool, overflight: bool) -> Kind {
    match (arr, dep) {
        (true, true) => Kind::Both,
        (true, false) => Kind::Arrival,
        (false, true) => Kind::Departure,
        _ if overflight => Kind::Overflight,
        _ => Kind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flights::Track;
    use crate::geo::to_runway;
    use crate::testkit::{approach, cruise, departure};
    use holdshort_core::airport::LTFM;

    fn rwy(name: &str) -> &'static RunwayEnd {
        LTFM.runways.iter().find(|r| r.name == name).unwrap()
    }

    fn track(points: Vec<holdshort_core::Obs>) -> Track {
        Track {
            hex: "test01".into(),
            points,
        }
    }

    fn no_wx() -> Weather {
        Weather::new(&[])
    }

    static SAW_06: RunwayEnd = RunwayEnd {
        name: "06",
        lat: 40.8986,
        lon: 29.3092,
        heading_deg: 60.0,
        elevation_ft: 312.0,
    };

    #[test]
    fn straight_in_35l() {
        let t = track(approach(rwy("35L"), 0.0, 10.0, 0));
        let fit = arrival_fit(&t, &LTFM, &no_wx()).expect("arrival");
        assert_eq!(fit.runway.name, "35L");
        assert!(fit.margin_m >= 150.0, "{}", fit.margin_m);
        assert!(fit.idx.len() >= 3);
        assert!(departure_fit(&t, &LTFM, &no_wx()).is_none());
    }

    /// Points on the line through the two real thresholds of 17L/35R,
    /// `nm` before the 35R threshold: independent of any heading value.
    fn true_line_35r(t0: i64) -> Vec<holdshort_core::Obs> {
        let (thr, far) = (rwy("35R"), rwy("17L"));
        let len_nm = holdshort_core::area::distance_nm(thr.lat, thr.lon, far.lat, far.lon);
        let trk = crate::geo::bearing(thr.lat, thr.lon, far.lat, far.lon);
        (0..60)
            .map(|k| {
                let nm = 6.0 - k as f64 * 0.09; // 6 nm .. ~0.7 nm out
                let f = -nm / len_nm;
                let (lat, lon) = (
                    thr.lat + f * (far.lat - thr.lat),
                    thr.lon + f * (far.lon - thr.lon),
                );
                let alt = thr.elevation_ft + nm * 1852.0 * 3f64.to_radians().tan() / 0.3048;
                crate::testkit::point(t0 + k * 5_000, lat, lon, alt, 140.0, trk, -700.0)
            })
            .collect()
    }

    #[test]
    fn true_centreline_35r() {
        let fit = arrival_fit(&track(true_line_35r(0)), &LTFM, &no_wx()).expect("arrival");
        assert_eq!(fit.runway.name, "35R");
    }

    #[test]
    fn parallel_200m_east_is_35r() {
        let t = track(approach(rwy("35R"), 0.0, 10.0, 0));
        assert_eq!(arrival_fit(&t, &LTFM, &no_wx()).unwrap().runway.name, "35R");
    }

    #[test]
    fn departure_from_36() {
        let t = track(departure(rwy("36"), 0));
        assert_eq!(
            departure_fit(&t, &LTFM, &no_wx()).unwrap().runway.name,
            "36"
        );
        assert!(arrival_fit(&t, &LTFM, &no_wx()).is_none());
    }

    #[test]
    fn overflight_fl360() {
        let t = track(cruise(36_000, 0));
        assert!(arrival_fit(&t, &LTFM, &no_wx()).is_none());
        assert!(departure_fit(&t, &LTFM, &no_wx()).is_none());
        assert!(is_overflight(&t, &LTFM, &no_wx()));
    }

    #[test]
    fn saw_arrival_is_other() {
        let t = track(approach(&SAW_06, 0.0, 10.0, 0));
        let (arr, dep) = (
            arrival_fit(&t, &LTFM, &no_wx()),
            departure_fit(&t, &LTFM, &no_wx()),
        );
        assert!(arr.is_none() && dep.is_none());
        assert!(!departs_seen_high(&t, &LTFM, &no_wx()));
        assert_eq!(
            kind(false, false, is_overflight(&t, &LTFM, &no_wx())),
            Kind::Other
        );
    }

    #[test]
    fn flight_without_heights_is_other() {
        let mut pts = approach(rwy("35L"), 0.0, 10.0, 0);
        for p in &mut pts {
            p.alt_baro_ft = None;
            p.alt_geom_ft = None;
        }
        let t = track(pts);
        assert!(arrival_fit(&t, &LTFM, &no_wx()).is_none());
        assert!(!is_overflight(&t, &LTFM, &no_wx()));
    }

    #[test]
    fn missing_track_is_not_aligned() {
        let mut pts = approach(rwy("35L"), 0.0, 10.0, 0);
        for p in &mut pts {
            p.track_deg = None;
        }
        assert!(arrival_fit(&track(pts), &LTFM, &no_wx()).is_none());
    }

    #[test]
    fn ambiguous_margin_reported() {
        let (l, r) = (rwy("17L"), rwy("17R"));
        let half = to_runway(l, r.lat, r.lon).cross_m / 2.0;
        let fit = arrival_fit(&track(approach(l, half, 10.0, 0)), &LTFM, &no_wx()).unwrap();
        assert!(fit.margin_m < 60.0, "{}", fit.margin_m);
    }

    #[test]
    fn kind_mapping() {
        assert_eq!(kind(true, false, false), Kind::Arrival);
        assert_eq!(kind(false, true, true), Kind::Departure);
        assert_eq!(kind(true, true, false), Kind::Both);
        assert_eq!(kind(false, false, true), Kind::Overflight);
        assert_eq!(kind(false, false, false), Kind::Other);
    }

    /// A departure first seen at 11,000 ft 12 nm out, already turned off the
    /// runway heading, climbing 2,000 ft/min away from the airport.
    fn high_departure() -> Track {
        use crate::testkit::legs;
        let lat = LTFM.lat + 12.0 / 60.0;
        let mut pts = legs(lat, LTFM.lon, 11_000.0, 300.0, &[(120.0, 0.0)], 0);
        for (k, p) in pts.iter_mut().enumerate() {
            let alt = 11_000.0 + 2000.0 * (k as f64 * 5.0 / 60.0);
            p.alt_baro_ft = Some(alt as i32);
            p.alt_geom_ft = Some(alt as i32);
            p.vrate_fpm = Some(2000);
        }
        track(pts)
    }

    #[test]
    fn departure_first_seen_high() {
        let t = high_departure();
        assert!(departure_fit(&t, &LTFM, &no_wx()).is_none());
        assert!(departs_seen_high(&t, &LTFM, &no_wx()));
    }

    #[test]
    fn arrival_or_descent_is_not_a_high_departure() {
        let t = track(approach(rwy("35L"), 0.0, 20.0, 0));
        assert!(!departs_seen_high(&t, &LTFM, &no_wx()));
        let mut far = high_departure();
        for p in &mut far.points {
            p.lat += 20.0 / 60.0; // first seen ~32 nm out
        }
        assert!(!departs_seen_high(&far, &LTFM, &no_wx()));
    }

    /// Spot-check finding: a light aircraft leaving Hezarfen (~9 nm SSW of
    /// LTFM) at 250 ft and 62 kt was counted as an LTFM departure.
    #[test]
    fn low_slow_nearby_airfield_departure_is_not_ltfm() {
        use crate::testkit::legs;
        let mut pts = legs(41.1095, 28.5653, 450.0, 150.0, &[(120.0, 0.0)], 0);
        for (k, p) in pts.iter_mut().enumerate() {
            let alt = 450.0 + 600.0 * (k as f64 * 5.0 / 60.0);
            p.alt_baro_ft = Some(alt as i32);
            p.alt_geom_ft = Some(alt as i32);
            p.vrate_fpm = Some(600);
            p.gs_kt = Some(70.0);
        }
        assert!(!departs_seen_high(&track(pts), &LTFM, &no_wx()));
    }

    #[test]
    fn climbing_towards_the_airport_is_not_a_departure() {
        let mut t = high_departure();
        t.points.reverse();
        let first_ts = t.points[0].ts_ms;
        for (k, p) in t.points.iter_mut().enumerate() {
            p.ts_ms = first_ts + k as i64 * 5_000;
        }
        // Same climb, but each point is closer to the airport than the last.
        let n = t.points.len();
        for k in 0..n {
            let alt = 11_000.0 + 2000.0 * (k as f64 * 5.0 / 60.0);
            t.points[k].alt_baro_ft = Some(alt as i32);
            t.points[k].alt_geom_ft = Some(alt as i32);
        }
        assert!(!departs_seen_high(&t, &LTFM, &no_wx()));
    }
}
