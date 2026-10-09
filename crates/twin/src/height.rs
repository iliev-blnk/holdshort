//! Height above a runway from pressure altitude corrected with the METAR QNH.

use holdshort_core::metar::{MetarFields, fields};
use holdshort_core::{Metar, Obs};

/// A METAR older than this is not used.
const MAX_AGE_MS: i64 = 7_200_000;
/// Feet of altitude per hPa of pressure difference near sea level.
const FT_PER_HPA: f64 = 27.3;
const STANDARD_HPA: f64 = 1013.25;

/// The day's METAR reports, decoded, oldest first.
pub struct Weather {
    reports: Vec<(i64, MetarFields)>,
}

impl Weather {
    pub fn new(metars: &[Metar]) -> Self {
        let mut reports: Vec<(i64, MetarFields)> = metars
            .iter()
            .map(|m| (m.obs_ts_ms, fields(&m.raw)))
            .collect();
        reports.sort_by_key(|r| r.0);
        Self { reports }
    }

    /// The report in effect at `t_ms`: the latest one not after it, if it is
    /// at most 2 h old.
    pub fn at(&self, t_ms: i64) -> Option<&MetarFields> {
        // `partition_point` is a binary search for the first report after t.
        let i = self.reports.partition_point(|r| r.0 <= t_ms);
        let (ts, f) = self.reports.get(i.checked_sub(1)?)?;
        (t_ms - ts <= MAX_AGE_MS).then_some(f)
    }
}

/// Height above a runway at `elevation_ft`: QNH-corrected pressure altitude,
/// else GPS altitude, else None.
pub fn h_agl(o: &Obs, wx: Option<&MetarFields>, elevation_ft: f64) -> Option<f64> {
    if let (Some(baro), Some(qnh)) = (o.alt_baro_ft, wx.and_then(|w| w.qnh_hpa)) {
        return Some(baro as f64 + (qnh - STANDARD_HPA) * FT_PER_HPA - elevation_ft);
    }
    o.alt_geom_ft.map(|g| g as f64 - elevation_ft)
}

#[cfg(test)]
mod tests {
    use super::*;
    use holdshort_core::{Metar, Obs, Src};

    fn metar(ts: i64, raw: &str) -> Metar {
        Metar {
            station: "LTFM".into(),
            obs_ts_ms: ts,
            raw: raw.into(),
        }
    }

    fn obs(baro: Option<i32>, geom: Option<i32>) -> Obs {
        Obs {
            ts_ms: 0,
            hex: "a".into(),
            flight: None,
            reg: None,
            ac_type: None,
            lat: 41.0,
            lon: 28.0,
            alt_baro_ft: baro,
            alt_geom_ft: geom,
            on_ground: false,
            gs_kt: None,
            track_deg: None,
            vrate_fpm: None,
            squawk: None,
            category: None,
            src: Src::History,
        }
    }

    #[test]
    fn weather_picks_latest_not_stale() {
        let wx = Weather::new(&[metar(1_800_000, "X 18010KT Q1020"), metar(0, "X Q1010")]);
        assert_eq!(wx.at(2_000_000).unwrap().qnh_hpa, Some(1020.0));
        assert_eq!(wx.at(1_000).unwrap().qnh_hpa, Some(1010.0));
        assert!(wx.at(1_800_000 + 7_200_001).is_none());
        assert!(Weather::new(&[]).at(5).is_none());
    }

    #[test]
    fn qnh_correction() {
        let wx = Weather::new(&[metar(0, "X Q1017")]);
        let h = h_agl(&obs(Some(1000), Some(5000)), wx.at(0), 0.0).unwrap();
        assert!((h - 1102.4).abs() < 0.5, "{h}");
    }

    #[test]
    fn geom_fallback() {
        assert_eq!(
            h_agl(&obs(Some(1000), Some(1200)), None, 300.0),
            Some(900.0)
        );
    }

    #[test]
    fn no_height() {
        assert_eq!(h_agl(&obs(None, None), None, 0.0), None);
        assert_eq!(h_agl(&obs(Some(1000), None), None, 0.0), None);
    }
}
