//! METAR from aviationweather.gov (live JSON) and Iowa Mesonet (history CSV).

use chrono::NaiveDateTime;
use serde::Deserialize;

use crate::{Error, Metar};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Awc {
    icao_id: String,
    obs_time: i64,
    raw_ob: String,
}

/// The fields of a raw METAR the twin engine needs.
#[derive(Debug, Clone, PartialEq)]
pub struct MetarFields {
    pub qnh_hpa: Option<f64>,
    /// Direction the wind blows from, true degrees; None when variable (VRB).
    pub wind_dir_deg: Option<f64>,
    /// Mean wind speed; gusts are ignored.
    pub wind_kt: Option<f64>,
}

fn digits(s: &str) -> Option<f64> {
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}

/// Pull QNH and wind out of a raw METAR by looking at each token.
pub fn fields(raw: &str) -> MetarFields {
    let mut f = MetarFields {
        qnh_hpa: None,
        wind_dir_deg: None,
        wind_kt: None,
    };
    for tok in raw.split_whitespace() {
        if tok.len() == 5
            && let Some(rest) = tok.strip_prefix('Q')
        {
            f.qnh_hpa = f.qnh_hpa.or(digits(rest));
        } else if tok.len() == 5
            && let Some(rest) = tok.strip_prefix('A')
        {
            // Inches of mercury in hundredths: A2992 = 29.92 inHg.
            f.qnh_hpa = f.qnh_hpa.or(digits(rest).map(|v| v / 100.0 * 33.8639));
        } else if let Some(body) = tok.strip_suffix("KT") {
            let body = body.split('G').next().unwrap_or(body);
            if body.len() < 5 || body.len() > 6 {
                continue;
            }
            let (dir, speed) = body.split_at(3);
            let Some(kt) = digits(speed) else { continue };
            let dir = if dir == "VRB" {
                None
            } else if let Some(d) = digits(dir) {
                Some(d)
            } else {
                continue;
            };
            f.wind_dir_deg = dir;
            f.wind_kt = Some(kt);
        }
    }
    f
}

/// aviationweather.gov `/api/data/metar?format=json`.
pub fn parse_awc(body: &str) -> Result<Vec<Metar>, Error> {
    let rows: Vec<Awc> = serde_json::from_str(body)?;
    Ok(rows
        .into_iter()
        .map(|r| Metar {
            station: r.icao_id,
            obs_ts_ms: r.obs_time * 1000,
            raw: r.raw_ob,
        })
        .collect())
}

/// Iowa Mesonet `asos.py?format=onlycomma`: `station,valid,metar`, UTC times.
/// Rows whose METAR is `M` or empty are skipped.
pub fn parse_iem_csv(body: &str) -> Result<Vec<Metar>, Error> {
    let mut out = Vec::new();
    for line in body.lines().skip(1) {
        // METAR text has no commas, but split at most twice to be safe.
        let mut cols = line.splitn(3, ',');
        let (Some(station), Some(valid), Some(raw)) = (cols.next(), cols.next(), cols.next())
        else {
            continue;
        };
        let raw = raw.trim();
        if raw.is_empty() || raw == "M" {
            continue;
        }
        let t = NaiveDateTime::parse_from_str(valid, "%Y-%m-%d %H:%M")
            .map_err(|e| Error::Format(format!("bad IEM time {valid:?}: {e}")))?;
        out.push(Metar {
            station: station.to_string(),
            obs_ts_ms: t.and_utc().timestamp_millis(),
            raw: raw.to_string(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AWC: &str = include_str!("../../../fixtures/metar_awc.json");
    const IEM: &str = include_str!("../../../fixtures/metar_iem.csv");

    #[test]
    fn awc_parses() {
        let m = parse_awc(AWC).unwrap();
        assert!(!m.is_empty());
        assert_eq!(m[0].station, "LTFM");
        assert!(m[0].raw.starts_with("METAR LTFM"), "{}", m[0].raw);
        assert_eq!(m[0].obs_ts_ms % 1000, 0);
    }

    #[test]
    fn iem_parses() {
        let m = parse_iem_csv(IEM).unwrap();
        assert_eq!(m[0].station, "LTFM");
        assert_eq!(m[0].obs_ts_ms, 1_791_332_400_000);
        assert_eq!(m[0].raw, "LTFM 070020Z VRB01KT CAVOK 14/11 Q1021 NOSIG");
    }

    #[test]
    fn iem_skips_missing() {
        assert_eq!(parse_iem_csv(IEM).unwrap().len(), 5);
    }

    #[test]
    fn metar_fields_qnh_and_altimeter() {
        assert_eq!(fields("Q1017").qnh_hpa, Some(1017.0));
        assert!((fields("A2992").qnh_hpa.unwrap() - 1013.2).abs() < 0.1);
    }

    #[test]
    fn metar_fields_wind() {
        let w = |s: &str| (fields(s).wind_dir_deg, fields(s).wind_kt);
        assert_eq!(w("20009KT"), (Some(200.0), Some(9.0)));
        assert_eq!(w("20009G19KT"), (Some(200.0), Some(9.0)));
        assert_eq!(w("VRB03KT"), (None, Some(3.0)));
        assert_eq!(w("00000KT"), (Some(0.0), Some(0.0)));
    }

    #[test]
    fn metar_fields_full_report() {
        let f = fields("METAR LTFM 091550Z 20009KT CAVOK 21/06 Q1017 NOSIG");
        assert_eq!(
            f,
            MetarFields {
                qnh_hpa: Some(1017.0),
                wind_dir_deg: Some(200.0),
                wind_kt: Some(9.0)
            }
        );
    }

    #[test]
    fn metar_fields_garbage() {
        assert_eq!(
            fields("hello world 12KT Q12"),
            MetarFields {
                qnh_hpa: None,
                wind_dir_deg: None,
                wind_kt: None
            }
        );
    }

    #[test]
    fn iem_bad_date_is_error() {
        assert!(parse_iem_csv("station,valid,metar\nLTFM,yesterday,LTFM 1\n").is_err());
    }
}
