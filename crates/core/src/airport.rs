//! The airports holdshort records, each a circle around its reference point.

use crate::Obs;
use crate::area::{EARTH_RADIUS_NM, distance_nm};

/// One end of a runway: its threshold, the heading flown when landing on it
/// (true degrees) and its elevation. From OurAirports `runways.csv` (public domain).
#[derive(Debug, PartialEq)]
pub struct RunwayEnd {
    pub name: &'static str,
    pub lat: f64,
    pub lon: f64,
    pub heading_deg: f64,
    pub elevation_ft: f64,
}

const LTFM_RUNWAYS: &[RunwayEnd] = &[
    RunwayEnd {
        name: "09",
        lat: 41.253693,
        lon: 28.766272,
        heading_deg: 89.0,
        elevation_ft: 274.0,
    },
    RunwayEnd {
        name: "27",
        lat: 41.254063,
        lon: 28.800018,
        heading_deg: 269.0,
        elevation_ft: 274.0,
    },
    RunwayEnd {
        name: "16L",
        lat: 41.29859924316406,
        lon: 28.709199905395508,
        heading_deg: 174.0,
        elevation_ft: 218.0,
    },
    RunwayEnd {
        name: "34R",
        lat: 41.26490020751953,
        lon: 28.70989990234375,
        heading_deg: 354.0,
        elevation_ft: 325.0,
    },
    RunwayEnd {
        name: "16R",
        lat: 41.29859924316406,
        lon: 28.706750869750977,
        heading_deg: 174.0,
        elevation_ft: 218.0,
    },
    RunwayEnd {
        name: "34L",
        lat: 41.26483917236328,
        lon: 28.70741081237793,
        heading_deg: 354.0,
        elevation_ft: 325.0,
    },
    RunwayEnd {
        name: "17L",
        lat: 41.298828125,
        lon: 28.727039337158203,
        heading_deg: 174.0,
        elevation_ft: 202.0,
    },
    RunwayEnd {
        name: "35R",
        lat: 41.26192092895508,
        lon: 28.727760314941406,
        heading_deg: 354.0,
        elevation_ft: 310.0,
    },
    RunwayEnd {
        name: "17R",
        lat: 41.29880142211914,
        lon: 28.72450065612793,
        heading_deg: 174.0,
        elevation_ft: 202.0,
    },
    RunwayEnd {
        name: "35L",
        lat: 41.26190185546875,
        lon: 28.725299835205078,
        heading_deg: 354.0,
        elevation_ft: 310.0,
    },
    RunwayEnd {
        name: "18",
        lat: 41.289798736572266,
        lon: 28.756200790405273,
        heading_deg: 179.0,
        elevation_ft: 221.0,
    },
    RunwayEnd {
        name: "36",
        lat: 41.26219940185547,
        lon: 28.75670051574707,
        heading_deg: 354.0,
        elevation_ft: 309.0,
    },
];

const LRSV_RUNWAYS: &[RunwayEnd] = &[
    RunwayEnd {
        name: "16",
        lat: 47.6952018737793,
        lon: 26.35059928894043,
        heading_deg: 163.0,
        elevation_ft: 1375.0,
    },
    RunwayEnd {
        name: "34",
        lat: 47.6797981262207,
        lon: 26.357500076293945,
        heading_deg: 343.0,
        elevation_ft: 1330.0,
    },
];

#[derive(Debug, PartialEq)]
pub struct Airport {
    pub icao: &'static str,
    pub name: &'static str,
    /// Aerodrome reference point.
    pub lat: f64,
    pub lon: f64,
    pub radius_nm: f64,
    pub runways: &'static [RunwayEnd],
}

pub const LTFM: Airport = Airport {
    icao: "LTFM",
    name: "Istanbul",
    lat: 41.2753,
    lon: 28.7519,
    radius_nm: 50.0,
    runways: LTFM_RUNWAYS,
};

pub const LRSV: Airport = Airport {
    icao: "LRSV",
    name: "Suceava",
    lat: 47.6875,
    lon: 26.3542,
    radius_nm: 50.0,
    runways: LRSV_RUNWAYS,
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

    /// Parse the OurAirports CSV subset (no embedded commas in these rows).
    fn csv_ends() -> Vec<(String, String, f64, f64, f64, f64)> {
        let text = include_str!("../../../fixtures/ourairports_runways.csv");
        let mut out = Vec::new();
        for line in text.lines().skip(1) {
            let c: Vec<String> = line
                .split(',')
                .map(|f| f.trim_matches('"').to_string())
                .collect();
            let num = |i: usize| c[i].parse::<f64>().unwrap();
            // le_ident 8, le lat 9, lon 10, elev 11, hdg 12; he_ident 14, lat 15, lon 16, elev 17, hdg 18
            out.push((
                c[2].clone(),
                c[8].clone(),
                num(9),
                num(10),
                num(12),
                num(11),
            ));
            out.push((
                c[2].clone(),
                c[14].clone(),
                num(15),
                num(16),
                num(18),
                num(17),
            ));
        }
        out
    }

    #[test]
    fn runways_match_ourairports() {
        let ends = csv_ends();
        for (icao, name, lat, lon, hdg, elev) in &ends {
            let ap = by_icao(icao).unwrap();
            let r = ap
                .runways
                .iter()
                .find(|r| r.name == name)
                .unwrap_or_else(|| panic!("{icao} {name}"));
            assert_eq!(
                (r.lat, r.lon, r.heading_deg, r.elevation_ft),
                (*lat, *lon, *hdg, *elev),
                "{icao} {name}"
            );
        }
        assert_eq!(LTFM.runways.len(), 12);
        assert_eq!(LRSV.runways.len(), 2);
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
