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
    fn iem_bad_date_is_error() {
        assert!(parse_iem_csv("station,valid,metar\nLTFM,yesterday,LTFM 1\n").is_err());
    }
}
