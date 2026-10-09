//! OpenSky `/api/states/all` responses (SI units, positional arrays).

use serde::Deserialize;
use serde_json::Value;

use crate::adsblol::clean;
use crate::{Error, Obs, Src};

/// (lamin, lomin, lamax, lomax): a box around the 50 nm circle.
pub const BBOX: (f64, f64, f64, f64) = (40.43, 27.64, 42.12, 29.86);

const M_TO_FT: f64 = 3.28084;
const MS_TO_KT: f64 = 1.943844;
const MS_TO_FPM: f64 = 196.8504;
const MAX_AGE_MS: i64 = 30_000;

#[derive(Deserialize)]
struct Response {
    // Required: present on every real response, absent on error bodies.
    #[allow(dead_code)]
    time: i64,
    // `null` when no aircraft are in the box.
    states: Option<Vec<Vec<Value>>>,
}

/// Parse one OpenSky response. Drops rows without a position or with a
/// position older than 30 s at `poll_ms`. Does not area-filter.
pub fn parse(body: &str, poll_ms: i64) -> Result<Vec<Obs>, Error> {
    let resp: Response = serde_json::from_str(body)?;
    Ok(resp
        .states
        .unwrap_or_default()
        .iter()
        .filter_map(|row| to_obs(row, poll_ms))
        .collect())
}

fn to_obs(row: &[Value], poll_ms: i64) -> Option<Obs> {
    // `.get(i)` returns None for short rows instead of panicking.
    let f = |i: usize| row.get(i).and_then(Value::as_f64);
    let s = |i: usize| row.get(i).and_then(Value::as_str).map(str::to_string);
    let ts_ms = (f(3)? * 1000.0).round() as i64;
    if poll_ms - ts_ms > MAX_AGE_MS {
        return None;
    }
    let (lon, lat) = (f(5)?, f(6)?);
    Some(Obs {
        ts_ms,
        hex: s(0)?.to_lowercase(),
        flight: clean(s(1)),
        reg: None,
        ac_type: None,
        lat,
        lon,
        // `as i32` truncates, so round first.
        alt_baro_ft: f(7).map(|m| (m * M_TO_FT).round() as i32),
        alt_geom_ft: f(13).map(|m| (m * M_TO_FT).round() as i32),
        on_ground: row.get(8).and_then(Value::as_bool).unwrap_or(false),
        gs_kt: f(9).map(|v| (v * MS_TO_KT) as f32),
        track_deg: f(10).map(|v| v as f32),
        vrate_fpm: f(11).map(|v| (v * MS_TO_FPM).round() as i32),
        squawk: clean(s(14)),
        category: row.get(17).and_then(Value::as_i64).map(|c| c.to_string()),
        src: Src::Opensky,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const POLL: i64 = 1_791_561_727_000;
    const SMALL: &str = include_str!("../../../fixtures/opensky_small.json");
    const REAL: &str = include_str!("../../../fixtures/opensky_ist.json");

    fn by_hex<'a>(obs: &'a [Obs], hex: &str) -> Option<&'a Obs> {
        obs.iter().find(|o| o.hex == hex)
    }

    #[test]
    fn converts_units() {
        let obs = parse(SMALL, POLL).unwrap();
        let f = by_hex(&obs, "4bc8d2").expect("F present");
        assert_eq!(f.ts_ms, 1_791_561_726_000);
        assert_eq!(f.alt_baro_ft, Some(22775));
        assert!((f.gs_kt.unwrap() - 408.0).abs() < 0.1);
        assert_eq!(f.vrate_fpm, Some(-1663));
        assert_eq!(f.alt_geom_ft, Some(23900));
        assert_eq!(f.flight.as_deref(), Some("PGT703B"));
        assert_eq!(f.squawk.as_deref(), Some("1410"));
        assert_eq!(f.category.as_deref(), Some("4"));
        assert_eq!(f.src, Src::Opensky);
    }

    #[test]
    fn ground_flag() {
        let obs = parse(SMALL, 1_791_561_700_000).unwrap();
        let g = by_hex(&obs, "4bb42b").expect("G present");
        assert!(g.on_ground);
        assert_eq!(g.alt_baro_ft, None);
    }

    #[test]
    fn stale_position_dropped() {
        // G's position is 35 s old at POLL.
        assert!(by_hex(&parse(SMALL, POLL).unwrap(), "4bb42b").is_none());
    }

    #[test]
    fn empty_callsign_is_none() {
        let obs = parse(SMALL, POLL).unwrap();
        assert_eq!(by_hex(&obs, "4ba9c1").expect("H present").flight, None);
    }

    #[test]
    fn null_position_time_dropped() {
        assert!(by_hex(&parse(SMALL, POLL).unwrap(), "4ba9c2").is_none());
    }

    #[test]
    fn null_states_is_empty() {
        assert!(
            parse(r#"{"time":1,"states":null}"#, POLL)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn error_body_is_error() {
        assert!(parse(r#"{"error":"Too many requests"}"#, POLL).is_err());
    }

    #[test]
    fn real_fixture_parses() {
        let time = serde_json::from_str::<serde_json::Value>(REAL).unwrap()["time"]
            .as_i64()
            .unwrap();
        assert!(!parse(REAL, time * 1000).unwrap().is_empty());
    }

    #[test]
    fn bbox_covers_circle() {
        use crate::area::{EARTH_RADIUS_NM, LTFM_LAT, LTFM_LON, RADIUS_NM};
        let dlat = (RADIUS_NM / EARTH_RADIUS_NM).to_degrees();
        let dlon = dlat / LTFM_LAT.to_radians().cos();
        let (lamin, lomin, lamax, lomax) = BBOX;
        assert!(lamin <= LTFM_LAT - dlat && lamax >= LTFM_LAT + dlat);
        assert!(lomin <= LTFM_LON - dlon && lomax >= LTFM_LON + dlon);
    }
}
