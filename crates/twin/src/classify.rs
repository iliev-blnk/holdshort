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
fn aligned(p: &Obs, r: &RunwayEnd, along: (f64, f64)) -> Option<f64> {
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

pub fn kind(arr: &Option<Fit>, dep: &Option<Fit>, overflight: bool) -> Kind {
    match (arr.is_some(), dep.is_some()) {
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
        assert_eq!(
            kind(&arr, &dep, is_overflight(&t, &LTFM, &no_wx())),
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
        let t = track(approach(rwy("35L"), 0.0, 10.0, 0));
        let a = arrival_fit(&t, &LTFM, &no_wx());
        assert_eq!(kind(&a, &None, false), Kind::Arrival);
        assert_eq!(kind(&None, &a, false), Kind::Departure);
        assert_eq!(kind(&a, &a, false), Kind::Both);
        assert_eq!(kind(&None, &None, true), Kind::Overflight);
    }
}
