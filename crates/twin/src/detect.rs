//! Times and events inside one flight: touchdown, liftoff, ASMA entry,
//! go-arounds and holding.

use holdshort_core::Obs;
use holdshort_core::airport::{Airport, RunwayEnd};
use holdshort_core::area::distance_nm;

use crate::classify::{Fit, aligned};
use crate::flights::Track;
use crate::geo::{NM_M, ring_crossing_ms, to_runway};
use crate::height::{Weather, h_agl};

const KT_MS: f64 = NM_M / 3600.0;
/// Touchdown is assumed this far past the threshold.
const TOUCHDOWN_ZONE_M: f64 = 300.0;
/// Liftoff is assumed this far past the threshold.
const LIFTOFF_M: f64 = 1_800.0;
const MIN_CLIMB_FPM: f64 = 300.0;
const LOW_CONFIDENCE_MS: i64 = 60_000;
pub const ASMA_RING_NM: f64 = 40.0;

fn speed_ms(p: &Obs) -> Option<f64> {
    p.gs_kt.map(|g| g as f64 * KT_MS).filter(|v| *v > 1.0)
}

/// Estimated touchdown time and whether the estimate is low-confidence:
/// the last point's distance to the touchdown zone at its ground speed.
pub fn touchdown(t: &Track, fit: &Fit) -> (i64, bool) {
    let last = t.points.last().expect("a fitted track has points");
    let Some(v) = speed_ms(last) else {
        return (last.ts_ms, true);
    };
    let along = to_runway(fit.runway, last.lat, last.lon).along_m;
    let dt = ((TOUCHDOWN_ZONE_M - along).max(0.0) / v * 1000.0).round() as i64;
    (last.ts_ms + dt, dt > LOW_CONFIDENCE_MS)
}

/// Estimated liftoff time and whether it is low-confidence: back from the
/// first point by height / climb rate, else by distance past the liftoff
/// point / ground speed.
pub fn liftoff(t: &Track, fit: &Fit, wx: &Weather) -> (i64, bool) {
    let first = t.points.first().expect("a fitted track has points");
    let h = h_agl(first, wx.at(first.ts_ms), fit.runway.elevation_ft);
    let climb = first
        .vrate_fpm
        .map(|v| v as f64)
        .filter(|v| *v >= MIN_CLIMB_FPM);
    let dt = match (h, climb) {
        (Some(h), Some(fpm)) => (h.max(0.0) / (fpm / 60.0) * 1000.0).round() as i64,
        _ => match speed_ms(first) {
            Some(v) => {
                let along = to_runway(fit.runway, first.lat, first.lon).along_m;
                ((along - LIFTOFF_M).max(0.0) / v * 1000.0).round() as i64
            }
            None => return (first.ts_ms, true),
        },
    };
    (first.ts_ms - dt, dt > LOW_CONFIDENCE_MS)
}

/// When the flight first crossed inward through the 40 nm ring; None when it
/// was first seen already inside.
pub fn asma_entry_ms(t: &Track, ap: &Airport) -> Option<i64> {
    let d = |p: &Obs| distance_nm(ap.lat, ap.lon, p.lat, p.lon);
    if d(t.points.first()?) <= ASMA_RING_NM {
        return None;
    }
    t.points
        .windows(2)
        .find_map(|w| ring_crossing_ms(w[0].ts_ms, d(&w[0]), w[1].ts_ms, d(&w[1]), ASMA_RING_NM))
}

const GA_BELOW_FT: f64 = 1_500.0;
const GA_CLIMB_FT: f64 = 500.0;
const GA_WINDOW_MS: i64 = 180_000;
const GA_NEAR_NM: f64 = 10.0;
const REARM_FT: f64 = 2_000.0;
const HOLD_AWAY_NM: f64 = 8.0;
const HOLD_WINDOW_MS: i64 = 360_000;
const HOLD_TURN_DEG: f64 = 300.0;

#[derive(Debug, Clone, PartialEq)]
pub struct GoAround {
    /// Time of the lowest point of the abandoned approach.
    pub ms: i64,
    pub runway: &'static str,
    pub min_h_ft: f64,
    pub lat: f64,
    pub lon: f64,
}

fn airport_elevation(ap: &Airport) -> f64 {
    ap.runways
        .iter()
        .map(|r| r.elevation_ft)
        .fold(f64::INFINITY, f64::min)
}

/// Approaches that were abandoned: lined up and descending below 1,500 ft,
/// then a climb of 500 ft or more within 3 minutes, still near the runway.
pub fn go_arounds(t: &Track, ap: &'static Airport, wx: &Weather) -> Vec<GoAround> {
    let pts = &t.points;
    let h = |p: &Obs, r: &RunwayEnd| h_agl(p, wx.at(p.ts_ms), r.elevation_ft);
    let elev = airport_elevation(ap);
    let mut out = Vec::new();
    let mut armed = true;
    let mut i = 0;
    // A labelled loop lets the inner search restart the outer one.
    'outer: while i < pts.len() {
        let p = &pts[i];
        if !armed {
            let far = distance_nm(ap.lat, ap.lon, p.lat, p.lon) > GA_NEAR_NM;
            let high = h_agl(p, wx.at(p.ts_ms), elev).is_some_and(|x| x >= REARM_FT);
            armed = far || high;
            i += 1;
            continue;
        }
        let descending = p.vrate_fpm.is_none_or(|v| v < 0);
        // The runway this point is lined up with: the closest centreline.
        let lined_up = ap
            .runways
            .iter()
            .filter_map(|r| aligned(p, r, (-6.0 * NM_M, 1_000.0)).map(|c| (r, c)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(r, _)| r);
        let start = lined_up.and_then(|r| {
            h(p, r)
                .filter(|hp| descending && *hp < GA_BELOW_FT)
                .map(|hp| (r, hp))
        });
        if let Some((r, hp)) = start {
            let (mut low, mut low_h) = (i, hp);
            let mut j = i + 1;
            while j < pts.len() && pts[j].ts_ms - pts[low].ts_ms <= GA_WINDOW_MS {
                if let Some(hj) = h(&pts[j], r) {
                    let near = distance_nm(r.lat, r.lon, pts[j].lat, pts[j].lon) <= GA_NEAR_NM;
                    if hj < low_h {
                        (low, low_h) = (j, hj);
                    } else if hj >= low_h + GA_CLIMB_FT && near {
                        let lp = &pts[low];
                        out.push(GoAround {
                            ms: lp.ts_ms,
                            runway: r.name,
                            min_h_ft: low_h,
                            lat: lp.lat,
                            lon: lp.lon,
                        });
                        armed = false;
                        i = j;
                        continue 'outer;
                    }
                }
                j += 1;
            }
        }
        i += 1;
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hold {
    pub start_ms: i64,
    pub end_ms: i64,
    pub laps: u32,
}

/// Signed change from track `a` to track `b`, in (-180, 180].
fn turn(a: f64, b: f64) -> f64 {
    let d = (b - a).rem_euclid(360.0);
    if d > 180.0 { d - 360.0 } else { d }
}

/// Holding: spans more than 8 nm out where the net turn in one direction
/// reaches 300° within 6 minutes.
pub fn holds(t: &Track, ap: &Airport) -> Vec<Hold> {
    let pts = &t.points;
    let n = pts.len();
    // cum[k] = net signed turn from point 0 to point k.
    let mut cum = vec![0.0; n];
    for k in 1..n {
        let d = match (pts[k - 1].track_deg, pts[k].track_deg) {
            (Some(a), Some(b)) => turn(a as f64, b as f64),
            _ => 0.0,
        };
        cum[k] = cum[k - 1] + d;
    }
    let away: Vec<bool> = pts
        .iter()
        .map(|p| distance_nm(ap.lat, ap.lon, p.lat, p.lon) > HOLD_AWAY_NM)
        .collect();
    let mut marked = vec![false; n];
    for i in 0..n {
        if !away[i] {
            continue;
        }
        let mut j = i;
        while j + 1 < n && away[j + 1] && pts[j + 1].ts_ms - pts[i].ts_ms <= HOLD_WINDOW_MS {
            j += 1;
        }
        if (cum[j] - cum[i]).abs() >= HOLD_TURN_DEG {
            marked[i..=j].iter_mut().for_each(|m| *m = true);
        }
    }
    let mut out = Vec::new();
    let mut k = 0;
    while k < n {
        if !marked[k] {
            k += 1;
            continue;
        }
        let start = k;
        while k + 1 < n && marked[k + 1] {
            k += 1;
        }
        let laps = ((cum[k] - cum[start]).abs() / 360.0).round().max(1.0) as u32;
        out.push(Hold {
            start_ms: pts[start].ts_ms,
            end_ms: pts[k].ts_ms,
            laps,
        });
        k += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::{arrival_fit, departure_fit};
    use crate::flights::Track;
    use crate::testkit::{approach, departure};
    use holdshort_core::airport::{LTFM, RunwayEnd};
    use holdshort_core::area::distance_nm;

    fn rwy(name: &str) -> &'static RunwayEnd {
        LTFM.runways.iter().find(|r| r.name == name).unwrap()
    }

    fn track(points: Vec<Obs>) -> Track {
        Track {
            hex: "test01".into(),
            points,
        }
    }

    #[test]
    fn touchdown_extrapolated() {
        let t = track(approach(rwy("35L"), 0.0, 10.0, 0));
        let fit = arrival_fit(&t, &LTFM, &Weather::new(&[])).unwrap();
        let (ms, low) = touchdown(&t, &fit);
        let last = t.points.last().unwrap().ts_ms;
        let expect = last + ((926.0 + 300.0) / (140.0 * 1852.0 / 3600.0) * 1000.0) as i64;
        assert!((ms - expect).abs() <= 1_000, "{ms} vs {expect}");
        assert!(!low);
    }

    #[test]
    fn no_ground_speed_touchdown_is_low_confidence() {
        let mut pts = approach(rwy("35L"), 0.0, 10.0, 0);
        pts.last_mut().unwrap().gs_kt = None;
        let t = track(pts);
        let fit = arrival_fit(&t, &LTFM, &Weather::new(&[])).unwrap();
        assert_eq!(touchdown(&t, &fit), (t.points.last().unwrap().ts_ms, true));
    }

    #[test]
    fn liftoff_from_climb() {
        let t = track(departure(rwy("36"), 100_000));
        let wx = Weather::new(&[]);
        let fit = departure_fit(&t, &LTFM, &wx).unwrap();
        let (ms, low) = liftoff(&t, &fit, &wx);
        assert!((ms - (100_000 - 18_000)).abs() <= 1_000, "{ms}");
        assert!(!low);
    }

    #[test]
    fn liftoff_fallback_without_climb() {
        let mut pts = departure(rwy("36"), 100_000);
        for p in &mut pts {
            p.vrate_fpm = None;
        }
        // Start further out (~2.8 km past the threshold, still below 1,500 ft).
        let t = track(pts.split_off(4));
        let wx = Weather::new(&[]);
        let fit = departure_fit(&t, &LTFM, &wx).unwrap();
        let first = &t.points[0];
        let along = crate::geo::to_runway(fit.runway, first.lat, first.lon).along_m;
        let expect = first.ts_ms - ((along - 1800.0) / (160.0 * 1852.0 / 3600.0) * 1000.0) as i64;
        let (ms, low) = liftoff(&t, &fit, &wx);
        assert!((ms - expect).abs() <= 1_000, "{ms} vs {expect}");
        assert!(!low);
    }

    #[test]
    fn asma_entry_interpolated() {
        let t = track(approach(rwy("35L"), 0.0, 45.0, 0));
        let ms = asma_entry_ms(&t, &LTFM).expect("entry");
        let i = t.points.iter().position(|p| p.ts_ms > ms).unwrap();
        let (a, b) = (&t.points[i - 1], &t.points[i]);
        let f = (ms - a.ts_ms) as f64 / (b.ts_ms - a.ts_ms) as f64;
        let (lat, lon) = (a.lat + f * (b.lat - a.lat), a.lon + f * (b.lon - a.lon));
        assert!((distance_nm(LTFM.lat, LTFM.lon, lat, lon) - 40.0).abs() < 0.05);
    }

    /// Approach to 35L down to ~300 ft, climb straight ahead to 3,000 ft, then
    /// a full approach to 34L.
    fn go_around_flight() -> Vec<Obs> {
        use crate::geo::from_runway;
        use crate::height::h_agl;
        use crate::testkit::point;
        let r = rwy("35L");
        let mut pts: Vec<Obs> = approach(r, 0.0, 6.0, 0)
            .into_iter()
            .take_while(|p| h_agl(p, None, r.elevation_ft).unwrap() >= 300.0)
            .collect();
        let last = pts.last().unwrap().clone();
        let mut along = to_runway(r, last.lat, last.lon).along_m;
        let mut h = last.alt_baro_ft.unwrap() as f64;
        let mut t = last.ts_ms;
        while h < r.elevation_ft + 3000.0 {
            along += 160.0 * 1852.0 / 3600.0 * 5.0;
            h += 2000.0 / 12.0;
            t += 5_000;
            let (lat, lon) = from_runway(r, along, 0.0);
            pts.push(point(t, lat, lon, h, 160.0, r.heading_deg, 2000.0));
        }
        pts.extend(approach(rwy("34L"), 0.0, 8.0, t + 360_000));
        pts
    }

    #[test]
    fn go_around_then_land_other_runway() {
        let t = track(go_around_flight());
        let wx = Weather::new(&[]);
        let gas = go_arounds(&t, &LTFM, &wx);
        assert_eq!(gas.len(), 1, "{gas:?}");
        assert_eq!(gas[0].runway, "35L");
        assert!(
            (gas[0].min_h_ft - 300.0).abs() < 50.0,
            "{}",
            gas[0].min_h_ft
        );
        assert_eq!(arrival_fit(&t, &LTFM, &wx).unwrap().runway.name, "34L");
    }

    #[test]
    fn normal_landing_no_go_around() {
        let t = track(approach(rwy("35L"), 0.0, 10.0, 0));
        assert!(go_arounds(&t, &LTFM, &Weather::new(&[])).is_empty());
    }

    #[test]
    fn two_lap_hold() {
        let pts = crate::testkit::racetrack(LTFM.lat + 12.0 / 60.0, LTFM.lon, 12_000.0, 2, 0);
        let hs = holds(&track(pts), &LTFM);
        assert_eq!(hs.len(), 1, "{hs:?}");
        assert_eq!(hs[0].laps, 2);
        assert!(
            ((hs[0].end_ms - hs[0].start_ms) - 480_000).abs() <= 30_000,
            "{hs:?}"
        );
    }

    #[test]
    fn pattern_is_not_a_hold() {
        assert!(holds(&track(crate::testkit::pattern(0)), &LTFM).is_empty());
    }

    #[test]
    fn hold_near_airport_ignored() {
        let pts = crate::testkit::racetrack(LTFM.lat + 5.0 / 60.0, LTFM.lon, 6_000.0, 2, 0);
        assert!(holds(&track(pts), &LTFM).is_empty());
    }

    #[test]
    fn duplicate_timestamps_do_not_break_holding() {
        let pts = crate::testkit::racetrack(LTFM.lat + 12.0 / 60.0, LTFM.lon, 12_000.0, 2, 0);
        let doubled: Vec<Obs> = pts.iter().flat_map(|p| [p.clone(), p.clone()]).collect();
        let hs = holds(&track(doubled), &LTFM);
        assert_eq!(hs.len(), 1);
        assert_eq!(hs[0].laps, 2);
    }

    #[test]
    fn asma_none_when_first_seen_inside() {
        assert!(asma_entry_ms(&track(approach(rwy("35L"), 0.0, 30.0, 0)), &LTFM).is_none());
    }
}
