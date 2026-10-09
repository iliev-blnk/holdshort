//! readsb `trace_full_<hex>.json` files from the adsb.lol daily history.

use serde::Deserialize;
use serde_json::Value;

use crate::adsblol::clean;
use crate::{Error, Obs, Src};

// Point layout: [dt, lat, lon, alt, gs, track, flags, vrate, aircraft|null,
//                source_type, alt_geom, geom_rate, ias, roll]
const STALE_FLAG: i64 = 1;

#[derive(Deserialize)]
struct Trace {
    icao: String,
    r: Option<String>,
    t: Option<String>,
    timestamp: f64,
    trace: Vec<Vec<Value>>,
}

/// Parse one decompressed trace file. Skips stale and malformed points.
/// Callsign, squawk and category appear only when they change, so they are
/// carried forward. Does not area-filter.
pub fn parse_trace(json: &str) -> Result<Vec<Obs>, Error> {
    let tr: Trace = serde_json::from_str(json)?;
    let hex = tr.icao.to_lowercase();
    let (reg, ac_type) = (clean(tr.r), clean(tr.t));
    let (mut flight, mut squawk, mut category) = (None, None, None);
    let mut out = Vec::with_capacity(tr.trace.len());
    for p in &tr.trace {
        if p.len() < 8 {
            continue;
        }
        let f = |i: usize| p.get(i).and_then(Value::as_f64);
        // Update carried-forward fields first: they belong to this point too.
        if let Some(ac) = p.get(8).and_then(Value::as_object) {
            let text = |k: &str| clean(ac.get(k).and_then(Value::as_str).map(str::to_string));
            flight = text("flight").or(flight);
            squawk = text("squawk").or(squawk);
            category = text("category").or(category);
        }
        if p[6].as_i64().unwrap_or(0) & STALE_FLAG != 0 {
            continue;
        }
        let (Some(dt), Some(lat), Some(lon)) = (f(0), f(1), f(2)) else {
            continue;
        };
        let (alt_baro_ft, on_ground) = match &p[3] {
            Value::String(s) if s == "ground" => (None, true),
            Value::Number(n) => (n.as_f64().map(|v| v.round() as i32), false),
            _ => (None, false),
        };
        out.push(Obs {
            ts_ms: ((tr.timestamp + dt) * 1000.0).round() as i64,
            hex: hex.clone(),
            flight: flight.clone(),
            reg: reg.clone(),
            ac_type: ac_type.clone(),
            lat,
            lon,
            alt_baro_ft,
            alt_geom_ft: f(10).map(|v| v.round() as i32),
            on_ground,
            gs_kt: f(4).map(|v| v as f32),
            track_deg: f(5).map(|v| v as f32),
            vrate_fpm: f(7).or(f(11)).map(|v| v.round() as i32),
            squawk: squawk.clone(),
            category: category.clone(),
            src: Src::History,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMALL: &str = include_str!("../../../fixtures/trace_small.json");
    const REAL: &str = include_str!("../../../fixtures/trace_full_sample.json");

    #[test]
    fn trace_timestamps() {
        let obs = parse_trace(SMALL).unwrap();
        assert_eq!(obs[0].ts_ms, 1_791_417_600_000);
        assert_eq!(obs.last().unwrap().ts_ms, 1_791_417_615_000);
    }

    #[test]
    fn trace_skips_stale_and_short() {
        assert_eq!(parse_trace(SMALL).unwrap().len(), 3);
    }

    #[test]
    fn trace_carries_callsign_forward() {
        let obs = parse_trace(SMALL).unwrap();
        assert_eq!(obs[1].flight.as_deref(), Some("THY1"));
        assert_eq!(obs[1].squawk.as_deref(), Some("2311"));
        assert_eq!(obs[1].category.as_deref(), Some("A3"));
        assert_eq!(obs[2].flight.as_deref(), Some("THY2"));
    }

    #[test]
    fn trace_ground() {
        let obs = parse_trace(SMALL).unwrap();
        assert!(!obs[0].on_ground);
        assert_eq!(obs[0].alt_baro_ft, Some(3000));
        assert_eq!(obs[0].alt_geom_ft, Some(3100));
        assert_eq!(obs[0].vrate_fpm, Some(-640));
        assert!(obs[1].on_ground);
        assert_eq!(obs[1].alt_baro_ft, None);
    }

    #[test]
    fn trace_reg_type() {
        let obs = parse_trace(SMALL).unwrap();
        assert!(obs.iter().all(|o| o.reg.as_deref() == Some("TC-LPH")));
        assert!(obs.iter().all(|o| o.ac_type.as_deref() == Some("A21N")));
        assert!(
            obs.iter()
                .all(|o| o.hex == "4bb208" && o.src == Src::History)
        );
    }

    #[test]
    fn real_trace_parses() {
        let obs = parse_trace(REAL).unwrap();
        assert!(!obs.is_empty());
        assert_eq!(obs[0].hex, "~2b9049");
    }

    #[test]
    fn not_a_trace_is_error() {
        assert!(parse_trace("{}").is_err());
    }
}
