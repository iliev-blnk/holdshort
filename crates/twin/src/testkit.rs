//! Synthetic trajectories with known truth, for tests. Heights are written
//! to both baro and GPS altitude, so they hold with or without a METAR.

use holdshort_core::airport::{LTFM, RunwayEnd};
use holdshort_core::{Obs, Src};

use crate::geo::{FT_M, NM_M, from_runway};

pub const STEP_MS: i64 = 5_000;
const KT_MS: f64 = NM_M / 3600.0;

pub fn point(
    ts_ms: i64,
    lat: f64,
    lon: f64,
    alt_ft: f64,
    gs_kt: f64,
    track: f64,
    vrate: f64,
) -> Obs {
    Obs {
        ts_ms,
        hex: "test01".into(),
        flight: Some("TST1".into()),
        reg: None,
        ac_type: Some("A320".into()),
        lat,
        lon,
        alt_baro_ft: Some(alt_ft.round() as i32),
        alt_geom_ft: Some(alt_ft.round() as i32),
        on_ground: false,
        gs_kt: Some(gs_kt as f32),
        track_deg: Some(track.rem_euclid(360.0) as f32),
        vrate_fpm: Some(vrate.round() as i32),
        squawk: None,
        category: None,
        src: Src::History,
    }
}

pub fn with_hex(mut v: Vec<Obs>, hex: &str) -> Vec<Obs> {
    for o in &mut v {
        o.hex = hex.into();
    }
    v
}

/// A 3° approach on `rwy`'s extended centreline shifted `cross_m` to the
/// right, from `from_nm` out to 0.5 nm before the threshold, 140 kt, 5 s steps.
pub fn approach(rwy: &RunwayEnd, cross_m: f64, from_nm: f64, t0: i64) -> Vec<Obs> {
    let gs = 140.0;
    let step_m = gs * KT_MS * STEP_MS as f64 / 1000.0;
    let glide = 3f64.to_radians().tan();
    let vrate = -(gs * KT_MS * glide) / FT_M * 60.0;
    let (start, end) = (-from_nm * NM_M, -0.5 * NM_M);
    let n = ((end - start) / step_m).floor() as i64;
    (0..=n)
        .map(|i| {
            // Count back from the end so the last point sits exactly at 0.5 nm.
            let along = end - (n - i) as f64 * step_m;
            let (lat, lon) = from_runway(rwy, along, cross_m);
            let alt = rwy.elevation_ft + (-along * glide) / FT_M;
            point(t0 + i * STEP_MS, lat, lon, alt, gs, rwy.heading_deg, vrate)
        })
        .collect()
}

/// A departure from `rwy`: from 0.6 nm past the threshold at 600 ft above
/// it, climbing 2,000 ft/min at 160 kt, out to 8 nm.
pub fn departure(rwy: &RunwayEnd, t0: i64) -> Vec<Obs> {
    let gs = 160.0;
    let step_m = gs * KT_MS * STEP_MS as f64 / 1000.0;
    let mut out = Vec::new();
    let mut along = 0.6 * NM_M;
    let mut h = 600.0;
    let mut t = t0;
    while along <= 8.0 * NM_M {
        let (lat, lon) = from_runway(rwy, along, 0.0);
        out.push(point(
            t,
            lat,
            lon,
            rwy.elevation_ft + h,
            gs,
            rwy.heading_deg,
            2000.0,
        ));
        along += step_m;
        h += 2000.0 * STEP_MS as f64 / 60_000.0;
        t += STEP_MS;
    }
    out
}

/// `laps` full right-hand circles of 4 minutes at 220 kt and `alt_ft`.
pub fn racetrack(center_lat: f64, center_lon: f64, alt_ft: f64, laps: u32, t0: i64) -> Vec<Obs> {
    let gs = 220.0;
    let lap_ms = 240_000.0;
    let r_m = gs * KT_MS * lap_ms / 1000.0 / std::f64::consts::TAU;
    let steps = (laps as f64 * lap_ms / STEP_MS as f64) as i64;
    (0..=steps)
        .map(|i| {
            let theta = std::f64::consts::TAU * (i * STEP_MS) as f64 / lap_ms;
            let (n, e) = (r_m * theta.cos(), r_m * theta.sin());
            let lat = center_lat + (n / 6_371_008.8).to_degrees();
            let lon = center_lon + (e / (6_371_008.8 * center_lat.to_radians().cos())).to_degrees();
            // Clockwise motion: the track is the position angle + 90°.
            point(
                t0 + i * STEP_MS,
                lat,
                lon,
                alt_ft,
                gs,
                theta.to_degrees() + 90.0,
                0.0,
            )
        })
        .collect()
}

/// Straight flight segments joined by turns, starting at (lat, lon) on
/// `track0`; each leg is (seconds, degrees turned over that leg).
pub fn legs(
    lat: f64,
    lon: f64,
    alt_ft: f64,
    track0: f64,
    legs: &[(f64, f64)],
    t0: i64,
) -> Vec<Obs> {
    let gs = 200.0;
    let step_m = gs * KT_MS * STEP_MS as f64 / 1000.0;
    let (mut lat, mut lon, mut trk, mut t) = (lat, lon, track0, t0);
    let mut out = vec![point(t, lat, lon, alt_ft, gs, trk, 0.0)];
    for &(secs, turn) in legs {
        let steps = (secs * 1000.0 / STEP_MS as f64).round() as i64;
        for _ in 0..steps {
            trk += turn / steps as f64;
            let (s, c) = trk.to_radians().sin_cos();
            lat += (step_m * c / 6_371_008.8).to_degrees();
            lon += (step_m * s / (6_371_008.8 * lat.to_radians().cos())).to_degrees();
            t += STEP_MS;
            out.push(point(t, lat, lon, alt_ft, gs, trk, 0.0));
        }
    }
    out
}

/// Downwind, base and final: about 180° of turning in total, 15 nm south of LTFM.
pub fn pattern(t0: i64) -> Vec<Obs> {
    let lat = LTFM.lat - 15.0 / 60.0;
    legs(
        lat,
        LTFM.lon,
        4000.0,
        174.0,
        &[
            (180.0, 0.0),
            (30.0, 90.0),
            (60.0, 0.0),
            (30.0, 90.0),
            (120.0, 0.0),
        ],
        t0,
    )
}

/// Straight across the circle from west to east at `alt_ft`, 450 kt.
pub fn cruise(alt_ft: i32, t0: i64) -> Vec<Obs> {
    let lon0 = LTFM.lon - 45.0 / (60.0 * LTFM.lat.to_radians().cos());
    let mut v = legs(LTFM.lat, lon0, alt_ft as f64, 90.0, &[(720.0, 0.0)], t0);
    for o in &mut v {
        o.gs_kt = Some(450.0);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::to_runway;
    use crate::height::h_agl;
    use holdshort_core::airport::LTFM;

    #[test]
    fn testkit_approach_ends_near_threshold() {
        let r = LTFM.runways.iter().find(|r| r.name == "35L").unwrap();
        let pts = approach(r, 0.0, 10.0, 0);
        let last = pts.last().unwrap();
        let l = to_runway(r, last.lat, last.lon);
        assert!((l.along_m + 926.0).abs() < 10.0, "{l:?}");
        let h = h_agl(last, None, r.elevation_ft).unwrap();
        assert!((h - 160.0).abs() < 20.0, "{h}");
        assert!(pts.windows(2).all(|w| w[1].ts_ms - w[0].ts_ms == 5_000));
    }
}
