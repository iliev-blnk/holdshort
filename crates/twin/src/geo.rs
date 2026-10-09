//! Runway-relative geometry in a flat local projection (fine within ~15 km).

pub use holdshort_core::airport::RunwayEnd;

pub const NM_M: f64 = 1852.0;
pub const FT_M: f64 = 0.3048;
const EARTH_M: f64 = 6_371_008.8;

/// A position relative to a runway end: `along_m` along the landing heading
/// from the threshold (negative = before it, on approach), `cross_m` to the
/// right of the extended centreline looking along the heading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Local {
    pub along_m: f64,
    pub cross_m: f64,
}

pub fn to_runway(r: &RunwayEnd, lat: f64, lon: f64) -> Local {
    let n = (lat - r.lat).to_radians() * EARTH_M;
    let e = (lon - r.lon).to_radians() * EARTH_M * r.lat.to_radians().cos();
    let (s, c) = r.heading_deg.to_radians().sin_cos();
    Local {
        along_m: e * s + n * c,
        cross_m: e * c - n * s,
    }
}

/// Inverse of `to_runway`.
pub fn from_runway(r: &RunwayEnd, along_m: f64, cross_m: f64) -> (f64, f64) {
    let (s, c) = r.heading_deg.to_radians().sin_cos();
    let e = along_m * s + cross_m * c;
    let n = along_m * c - cross_m * s;
    (
        r.lat + (n / EARTH_M).to_degrees(),
        r.lon + (e / (EARTH_M * r.lat.to_radians().cos())).to_degrees(),
    )
}

/// Smallest angle between two headings, in [0, 180].
pub fn heading_diff(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

/// Time an inbound aircraft crossed the `ring_nm` circle between two points
/// at distances `d0_nm` (outside) and `d1_nm` (inside or on it).
pub fn ring_crossing_ms(t0: i64, d0_nm: f64, t1: i64, d1_nm: f64, ring_nm: f64) -> Option<i64> {
    if !(d0_nm > ring_nm && d1_nm <= ring_nm) {
        return None;
    }
    let f = (d0_nm - ring_nm) / (d0_nm - d1_nm);
    Some(t0 + ((t1 - t0) as f64 * f).round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use holdshort_core::airport::LTFM;
    use holdshort_core::area::distance_nm;

    fn rwy(name: &str) -> &'static RunwayEnd {
        LTFM.runways.iter().find(|r| r.name == name).unwrap()
    }

    #[test]
    fn point_on_extended_centreline() {
        let r = rwy("35L");
        // 2 nm before the threshold: bearing 174° (heading 354 reversed).
        let b = 174f64.to_radians();
        let (lat, lon) = (
            r.lat + (2.0 * NM_M * b.cos()) / 111_195.0,
            r.lon + (2.0 * NM_M * b.sin()) / (111_195.0 * r.lat.to_radians().cos()),
        );
        assert!((distance_nm(r.lat, r.lon, lat, lon) - 2.0).abs() < 0.01);
        let l = to_runway(r, lat, lon);
        assert!((l.along_m + 3704.0).abs() < 5.0, "{l:?}");
        assert!(l.cross_m.abs() < 5.0, "{l:?}");
    }

    #[test]
    fn point_east_is_right_of_northbound() {
        let r = rwy("35L");
        let lon = r.lon + 100.0 / (111_195.0 * r.lat.to_radians().cos());
        let l = to_runway(r, r.lat, lon);
        assert!((l.cross_m - 99.45).abs() < 2.0, "{l:?}");
    }

    #[test]
    fn from_runway_inverts_to_runway() {
        let r = rwy("16R");
        let (lat, lon) = from_runway(r, -5000.0, 120.0);
        let l = to_runway(r, lat, lon);
        assert!(
            (l.along_m + 5000.0).abs() < 0.5 && (l.cross_m - 120.0).abs() < 0.5,
            "{l:?}"
        );
    }

    #[test]
    fn heading_diff_wraps() {
        assert_eq!(heading_diff(350.0, 10.0), 20.0);
        assert_eq!(heading_diff(10.0, 350.0), 20.0);
        assert_eq!(heading_diff(0.0, 180.0), 180.0);
    }

    #[test]
    fn ring_crossing_interpolates() {
        assert_eq!(ring_crossing_ms(0, 42.0, 10_000, 38.0, 40.0), Some(5_000));
    }

    #[test]
    fn ring_crossing_outward_is_none() {
        assert_eq!(ring_crossing_ms(0, 38.0, 10_000, 42.0, 40.0), None);
        assert_eq!(ring_crossing_ms(0, 45.0, 10_000, 41.0, 40.0), None);
    }

    #[test]
    fn ring_crossing_same_time() {
        assert_eq!(ring_crossing_ms(7, 42.0, 7, 38.0, 40.0), Some(7));
    }
}
