//! The airports holdshort records, each a circle around its reference point.

use crate::Obs;
use crate::area::{EARTH_RADIUS_NM, distance_nm};

#[derive(Debug, PartialEq)]
pub struct Airport {
    pub icao: &'static str,
    pub name: &'static str,
    /// Aerodrome reference point.
    pub lat: f64,
    pub lon: f64,
    pub radius_nm: f64,
}

pub const LTFM: Airport = Airport {
    icao: "LTFM",
    name: "Istanbul",
    lat: 41.2753,
    lon: 28.7519,
    radius_nm: 50.0,
};

pub const LRSV: Airport = Airport {
    icao: "LRSV",
    name: "Suceava",
    lat: 47.6875,
    lon: 26.3542,
    radius_nm: 50.0,
};

pub const AIRPORTS: &[&Airport] = &[&LTFM, &LRSV];

/// Look up a known airport by ICAO code, ignoring case.
pub fn by_icao(icao: &str) -> Option<&'static Airport> {
    AIRPORTS
        .iter()
        .copied()
        .find(|a| a.icao.eq_ignore_ascii_case(icao))
}

impl Airport {
    /// True when the point is within `radius_nm` of the reference point.
    pub fn contains(&self, lat: f64, lon: f64) -> bool {
        distance_nm(self.lat, self.lon, lat, lon) <= self.radius_nm
    }

    /// Drop observations outside this airport's circle, in place.
    pub fn retain(&self, obs: &mut Vec<Obs>) {
        // `retain` keeps items where the closure returns true; no new allocation.
        obs.retain(|o| self.contains(o.lat, o.lon));
    }

    /// (lamin, lomin, lamax, lomax) around the circle, rounded outward to 0.01°.
    pub fn bbox(&self) -> (f64, f64, f64, f64) {
        let dlat = (self.radius_nm / EARTH_RADIUS_NM).to_degrees();
        let dlon = dlat / self.lat.to_radians().cos();
        let down = |x: f64| (x * 100.0).floor() / 100.0;
        let up = |x: f64| (x * 100.0).ceil() / 100.0;
        (
            down(self.lat - dlat),
            down(self.lon - dlon),
            up(self.lat + dlat),
            up(self.lon + dlon),
        )
    }

    pub fn adsblol_url(&self) -> String {
        format!(
            "https://api.adsb.lol/v2/point/{}/{}/{:.0}",
            self.lat, self.lon, self.radius_nm
        )
    }

    pub fn opensky_url(&self) -> String {
        let (lamin, lomin, lamax, lomax) = self.bbox();
        format!(
            "https://opensky-network.org/api/states/all?lamin={lamin}&lomin={lomin}&lamax={lamax}&lomax={lomax}"
        )
    }

    pub fn awc_url(&self) -> String {
        format!(
            "https://aviationweather.gov/api/data/metar?ids={}&format=json&hours=1",
            self.icao
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::area::EARTH_RADIUS_NM;
    use crate::{Obs, Src};
    use std::f64::consts::PI;

    fn nm_north(a: &Airport, nm: f64) -> f64 {
        a.lat + nm * 180.0 / (PI * EARTH_RADIUS_NM)
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
    fn reference_points_inside() {
        for a in AIRPORTS {
            assert!(a.contains(a.lat, a.lon), "{}", a.icao);
        }
    }

    #[test]
    fn circle_edge_is_50_nm() {
        for a in AIRPORTS {
            assert!(a.contains(nm_north(a, 49.9), a.lon), "{}", a.icao);
            assert!(!a.contains(nm_north(a, 50.1), a.lon), "{}", a.icao);
        }
    }

    #[test]
    fn saw_is_in_ltfm_circle() {
        assert!(LTFM.contains(40.8986, 29.3092));
    }

    #[test]
    fn iasi_is_outside_lrsv_circle() {
        assert!(!LRSV.contains(47.1785, 27.6206));
    }

    #[test]
    fn circles_do_not_overlap() {
        assert!(!LTFM.contains(LRSV.lat, LRSV.lon));
        assert!(!LRSV.contains(LTFM.lat, LTFM.lon));
    }

    #[test]
    fn by_icao_ignores_case() {
        assert_eq!(by_icao("lrsv").map(|a| a.icao), Some("LRSV"));
        assert_eq!(by_icao("LTFM").map(|a| a.icao), Some("LTFM"));
        assert!(by_icao("EGLL").is_none());
    }

    #[test]
    fn bbox_covers_circle() {
        for a in AIRPORTS {
            let dlat = (a.radius_nm / EARTH_RADIUS_NM).to_degrees();
            let dlon = dlat / a.lat.to_radians().cos();
            let (lamin, lomin, lamax, lomax) = a.bbox();
            assert!(lamin <= a.lat - dlat && lamax >= a.lat + dlat, "{}", a.icao);
            assert!(lomin <= a.lon - dlon && lomax >= a.lon + dlon, "{}", a.icao);
        }
    }

    #[test]
    fn urls_use_the_airport() {
        assert_eq!(
            LRSV.adsblol_url(),
            "https://api.adsb.lol/v2/point/47.6875/26.3542/50"
        );
        assert!(LRSV.opensky_url().contains("lamin="));
        assert!(LRSV.awc_url().contains("ids=LRSV"));
    }

    #[test]
    fn retain_keeps_only_this_airport() {
        let mut v = vec![obs_at(LTFM.lat, LTFM.lon), obs_at(LRSV.lat, LRSV.lon)];
        LRSV.retain(&mut v);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].lat, LRSV.lat);
    }
}
