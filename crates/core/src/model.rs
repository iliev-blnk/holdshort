//! Record types written to R2 as tagged NDJSON lines.

// `derive(Serialize, Deserialize)` generates the JSON code at compile time.
use serde::{Deserialize, Serialize};

use crate::Error;

/// Where an observation came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Src {
    Adsblol,
    Opensky,
    History,
}

/// One aircraft position. Aviation units: ft, kt, ft/min.
// `Option<T>` is Rust's nullable: `Some(value)` or `None`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Obs {
    pub ts_ms: i64,
    pub hex: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flight: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ac_type: Option<String>,
    pub lat: f64,
    pub lon: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt_baro_ft: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt_geom_ft: Option<i32>,
    pub on_ground: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gs_kt: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_deg: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vrate_fpm: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub squawk: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub src: Src,
}

/// Status of one poll of a live feed. Gaps are data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Poll {
    pub ts_ms: i64,
    pub src: Src,
    pub ok: bool,
    pub count: u32,
    pub latency_ms: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A raw METAR report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metar {
    pub station: String,
    pub obs_ts_ms: i64,
    pub raw: String,
}

/// One NDJSON line, tagged `{"k":"obs"|"poll"|"metar", ...}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "k", rename_all = "lowercase")]
pub enum Line {
    Obs(Obs),
    Poll(Poll),
    Metar(Metar),
}

/// Encode lines as NDJSON, one JSON object per line, each ending in `\n`.
pub fn to_ndjson(lines: &[Line]) -> String {
    let mut out = String::new();
    for line in lines {
        // Serialising our own types cannot fail (no maps with non-string keys).
        out.push_str(&serde_json::to_string(line).expect("serialise line"));
        out.push('\n');
    }
    out
}

/// Decode NDJSON, skipping blank lines.
pub fn from_ndjson(s: &str) -> Result<Vec<Line>, Error> {
    s.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).map_err(Error::from))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_obs() -> Obs {
        Obs {
            ts_ms: 1_791_561_731_002,
            hex: "4bb208".into(),
            flight: Some("THY6VU".into()),
            reg: Some("TC-LPH".into()),
            ac_type: Some("A21N".into()),
            lat: 40.951445,
            lon: 28.03093,
            alt_baro_ft: Some(19550),
            alt_geom_ft: Some(20475),
            on_ground: false,
            gs_kt: Some(385.9),
            track_deg: Some(263.0),
            vrate_fpm: Some(1792),
            squawk: Some("2311".into()),
            category: Some("A3".into()),
            src: Src::Adsblol,
        }
    }

    fn bare_obs() -> Obs {
        Obs {
            ts_ms: 1,
            hex: "abc123".into(),
            flight: None,
            reg: None,
            ac_type: None,
            lat: 41.0,
            lon: 28.0,
            alt_baro_ft: None,
            alt_geom_ft: None,
            on_ground: true,
            gs_kt: None,
            track_deg: None,
            vrate_fpm: None,
            squawk: None,
            category: None,
            src: Src::History,
        }
    }

    #[test]
    fn ndjson_round_trip() {
        let lines = vec![
            Line::Obs(full_obs()),
            Line::Poll(Poll {
                ts_ms: 5,
                src: Src::Opensky,
                ok: false,
                count: 0,
                latency_ms: 5000,
                error: Some("timeout".into()),
            }),
            Line::Metar(Metar {
                station: "LTFM".into(),
                obs_ts_ms: 1_791_561_000_000,
                raw: "METAR LTFM 091550Z 20009KT CAVOK 21/06 Q1017 NOSIG".into(),
            }),
        ];
        let text = to_ndjson(&lines);
        assert_eq!(from_ndjson(&text).unwrap(), lines);
    }

    #[test]
    fn obs_line_is_tagged_and_sparse() {
        let text = to_ndjson(&[Line::Obs(bare_obs())]);
        assert!(text.starts_with(r#"{"k":"obs""#), "{text}");
        assert!(!text.contains("\"flight\""), "{text}");
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn src_serialises_lowercase() {
        assert_eq!(serde_json::to_string(&Src::Adsblol).unwrap(), "\"adsblol\"");
    }

    #[test]
    fn bad_line_is_error() {
        assert!(from_ndjson("{\"k\":\"nope\"}\n").is_err());
    }

    #[test]
    fn blank_lines_are_skipped() {
        let text = format!("\n{}\n", to_ndjson(&[Line::Obs(bare_obs())]));
        assert_eq!(from_ndjson(&text).unwrap().len(), 1);
    }
}
