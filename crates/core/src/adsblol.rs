//! adsb.lol `/v2/point/{lat}/{lon}/{radius}` responses (readsb JSON).

use serde::Deserialize;
use serde_json::Value;

use crate::{Error, Obs, Src};

/// Positions older than this at poll time are dropped.
pub const MAX_SEEN_POS_S: f64 = 30.0;

#[derive(Deserialize)]
struct Response {
    // Required: an error body without `ac` must fail the poll, not look empty.
    ac: Vec<Aircraft>,
    now: Option<f64>,
}

// Only the fields we keep; serde ignores the rest.
#[derive(Deserialize)]
struct Aircraft {
    hex: String,
    flight: Option<String>,
    r: Option<String>,
    t: Option<String>,
    // A number in feet, or the string "ground".
    alt_baro: Option<Value>,
    alt_geom: Option<f64>,
    gs: Option<f32>,
    track: Option<f32>,
    baro_rate: Option<f64>,
    geom_rate: Option<f64>,
    squawk: Option<String>,
    category: Option<String>,
    lat: Option<f64>,
    lon: Option<f64>,
    seen_pos: Option<f64>,
}

/// Parse one adsb.lol response. Reference time is the body's `now` (ms) when
/// present, else `poll_ms`. Does not area-filter.
pub fn parse(body: &str, poll_ms: i64) -> Result<Vec<Obs>, Error> {
    let resp: Response = serde_json::from_str(body)?;
    let now = resp.now.map(|n| n.round() as i64).unwrap_or(poll_ms);
    Ok(resp.ac.into_iter().filter_map(|a| to_obs(a, now)).collect())
}

fn to_obs(a: Aircraft, now: i64) -> Option<Obs> {
    // `?` on an Option returns None early: no position, no observation.
    let (lat, lon) = (a.lat?, a.lon?);
    let seen_pos = a.seen_pos.unwrap_or(0.0);
    if seen_pos > MAX_SEEN_POS_S {
        return None;
    }
    let (alt_baro_ft, on_ground) = match &a.alt_baro {
        Some(Value::String(s)) if s == "ground" => (None, true),
        Some(Value::Number(n)) => (n.as_f64().map(|f| f.round() as i32), false),
        _ => (None, false),
    };
    Some(Obs {
        ts_ms: now - (seen_pos * 1000.0).round() as i64,
        hex: a.hex.to_lowercase(),
        flight: clean(a.flight),
        reg: clean(a.r),
        ac_type: clean(a.t),
        lat,
        lon,
        alt_baro_ft,
        alt_geom_ft: a.alt_geom.map(|f| f.round() as i32),
        on_ground,
        gs_kt: a.gs,
        track_deg: a.track,
        vrate_fpm: a.baro_rate.or(a.geom_rate).map(|f| f.round() as i32),
        squawk: clean(a.squawk),
        category: clean(a.category),
        src: Src::Adsblol,
    })
}

/// Trim, and turn empty strings into None.
pub(crate) fn clean(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_791_561_733_502;
    // `include_str!` embeds the file at compile time: tests need no file I/O.
    const SMALL: &str = include_str!("../../../fixtures/adsblol_small.json");
    const REAL: &str = include_str!("../../../fixtures/adsblol_ist.json");

    fn small() -> Vec<Obs> {
        parse(SMALL, 0).unwrap()
    }

    fn by_hex<'a>(obs: &'a [Obs], hex: &str) -> Option<&'a Obs> {
        obs.iter().find(|o| o.hex == hex)
    }

    #[test]
    fn parses_airborne() {
        let obs = small();
        let a = by_hex(&obs, "4bb208").expect("A present");
        assert_eq!(a.flight.as_deref(), Some("THY6VU"));
        assert_eq!(a.ts_ms, 1_791_561_731_002);
        assert_eq!(a.alt_baro_ft, Some(19550));
        assert!(!a.on_ground);
        assert_eq!(a.vrate_fpm, Some(1792));
        assert_eq!(a.reg.as_deref(), Some("TC-LPH"));
        assert_eq!(a.ac_type.as_deref(), Some("A21N"));
        assert_eq!(a.src, Src::Adsblol);
    }

    #[test]
    fn ground_altitude_sets_on_ground() {
        let obs = small();
        let b = by_hex(&obs, "4bb42b").expect("B present");
        assert!(b.on_ground);
        assert_eq!(b.alt_baro_ft, None);
    }

    #[test]
    fn drops_stale_position() {
        assert!(by_hex(&small(), "4bc8d2").is_none());
    }

    #[test]
    fn drops_aircraft_without_position() {
        assert!(by_hex(&small(), "500213").is_none());
    }

    #[test]
    fn missing_seen_pos_uses_now() {
        let obs = small();
        let e = by_hex(&obs, "~2b9049").expect("E present");
        assert_eq!(e.ts_ms, NOW);
        assert_eq!(e.vrate_fpm, Some(-576), "falls back to geom_rate");
    }

    #[test]
    fn keeps_three_of_five() {
        assert_eq!(small().len(), 3);
    }

    #[test]
    fn poll_time_used_without_now() {
        let body = r#"{"ac":[{"hex":"aaaaaa","lat":41.0,"lon":28.0,"seen_pos":1.0}]}"#;
        assert_eq!(parse(body, 10_000).unwrap()[0].ts_ms, 9_000);
    }

    #[test]
    fn real_fixture_parses() {
        let obs = parse(REAL, 0).unwrap();
        assert!(!obs.is_empty());
        assert!(obs.iter().all(|o| o.hex == o.hex.to_lowercase()));
    }

    #[test]
    fn error_body_is_error() {
        assert!(parse("{}", 0).is_err());
        assert!(parse(r#"{"msg":"rate limited"}"#, 0).is_err());
    }

    #[test]
    fn no_aircraft_is_ok() {
        assert!(parse(r#"{"ac":[],"now":1}"#, 0).unwrap().is_empty());
    }

    #[test]
    fn garbage_is_error() {
        assert!(parse("<html>", 0).is_err());
    }
}
