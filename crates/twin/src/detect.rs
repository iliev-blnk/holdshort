//! Times and events inside one flight: touchdown, liftoff, ASMA entry,
//! go-arounds and holding.

use holdshort_core::Obs;
use holdshort_core::airport::Airport;
use holdshort_core::area::distance_nm;

use crate::classify::Fit;
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

    #[test]
    fn asma_none_when_first_seen_inside() {
        assert!(asma_entry_ms(&track(approach(rwy("35L"), 0.0, 30.0, 0)), &LTFM).is_none());
    }
}
