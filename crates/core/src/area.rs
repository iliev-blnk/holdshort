//! The 50 nm circle around Istanbul Airport (LTFM) that holdshort records.

use crate::Obs;

/// LTFM aerodrome reference point.
pub const LTFM_LAT: f64 = 41.2753;
pub const LTFM_LON: f64 = 28.7519;
pub const RADIUS_NM: f64 = 50.0;
pub const EARTH_RADIUS_NM: f64 = 3440.065;

/// Great-circle distance in nautical miles (haversine).
pub fn distance_nm(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dp = p2 - p1;
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_NM * a.sqrt().asin()
}

/// True when the point is within `RADIUS_NM` of the LTFM reference point.
pub fn in_area(lat: f64, lon: f64) -> bool {
    distance_nm(LTFM_LAT, LTFM_LON, lat, lon) <= RADIUS_NM
}

/// Drop observations outside the area, in place.
pub fn retain_in_area(obs: &mut Vec<Obs>) {
    // `retain` keeps items where the closure returns true; no new allocation.
    obs.retain(|o| in_area(o.lat, o.lon));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Obs, Src};
    use std::f64::consts::PI;

    fn nm_north(nm: f64) -> f64 {
        LTFM_LAT + nm * 180.0 / (PI * EARTH_RADIUS_NM)
    }

    fn obs_at(lat: f64, lon: f64) -> Obs {
        Obs {
            ts_ms: 0,
            hex: "abc123".into(),
            flight: None,
            reg: None,
            ac_type: None,
            lat,
            lon,
            alt_baro_ft: None,
            alt_geom_ft: None,
            on_ground: false,
            gs_kt: None,
            track_deg: None,
            vrate_fpm: None,
            squawk: None,
            category: None,
            src: Src::Adsblol,
        }
    }

    #[test]
    fn arp_is_inside() {
        assert!(in_area(LTFM_LAT, LTFM_LON));
    }

    #[test]
    fn just_inside_north() {
        assert!(in_area(nm_north(49.9), LTFM_LON));
    }

    #[test]
    fn just_outside_north() {
        assert!(!in_area(nm_north(50.1), LTFM_LON));
    }

    #[test]
    fn saw_is_inside() {
        assert!(in_area(40.8986, 29.3092));
    }

    #[test]
    fn distance_symmetric() {
        let a = distance_nm(LTFM_LAT, LTFM_LON, 40.8986, 29.3092);
        let b = distance_nm(40.8986, 29.3092, LTFM_LAT, LTFM_LON);
        assert!((a - b).abs() < 1e-9);
    }

    #[test]
    fn retain_drops_outside() {
        let mut v = vec![obs_at(LTFM_LAT, LTFM_LON), obs_at(0.0, 0.0)];
        retain_in_area(&mut v);
        assert_eq!(v.len(), 1);
    }
}
