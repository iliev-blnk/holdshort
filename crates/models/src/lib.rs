//! The forecasting models (sub-project 4), served with ONNX Runtime.
//!
//! The models are LightGBM regressors trained in `research/` and exported to ONNX.
//! This crate is native-only (ONNX Runtime is a C++ library); `core` and `twin` stay
//! free of it so they keep building for WebAssembly.

use std::f64::consts::TAU;
use std::path::Path;

use anyhow::{Context, bail, ensure};
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Timelike};
use holdshort_twin::features::{ETA_FEATURES, ETA_N};
use ort::session::Session;
use ort::value::Tensor;
use serde::Deserialize;

pub const HOURLY_N: usize = 9;
/// Column order of hourly rows (must match research/holdshort_research/hourly.py).
pub const HOURLY_FEATURES: [&str; HOURLY_N] = [
    "hour_sin",
    "hour_cos",
    "dow",
    "lag1",
    "lag2",
    "lag3",
    "same_yday",
    "same_week",
    "profile",
];

/// `models/meta.json`, written by `python -m holdshort_research all`.
#[derive(Debug, Deserialize)]
pub struct Meta {
    pub eta_features: Vec<String>,
    pub hourly_features: Vec<String>,
    /// Mean arrivals per UTC hour of day over the training days (NaN-free in practice).
    #[serde(deserialize_with = "profile24")]
    pub profile: [f32; 24],
    /// Training median of the landing time, seconds (the global-median baseline).
    pub eta_global_median_s: f64,
}

fn profile24<'de, D: serde::Deserializer<'de>>(d: D) -> Result<[f32; 24], D::Error> {
    let v: Vec<Option<f32>> = Vec::deserialize(d)?;
    let v: Vec<f32> = v.into_iter().map(|x| x.unwrap_or(f32::NAN)).collect();
    v.try_into()
        .map_err(|_| serde::de::Error::custom("profile must have 24 values"))
}

impl Meta {
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        let p = dir.join("meta.json");
        let text =
            std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
        Ok(serde_json::from_str(&text)?)
    }
}

fn session(path: &Path) -> anyhow::Result<Session> {
    Session::builder()?
        .commit_from_file(path)
        .with_context(|| format!("loading {}", path.display()))
}

/// Run a model whose input is `input` [N, F] float32 and whose first output is [N, 1].
fn run<const F: usize>(s: &mut Session, rows: &[[f32; F]]) -> anyhow::Result<Vec<f32>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let flat: Vec<f32> = rows.iter().flatten().copied().collect();
    let input = Tensor::from_array(([rows.len(), F], flat))?;
    let out = s.run(ort::inputs!["input" => input])?;
    let (_, y) = out[0].try_extract_tensor::<f32>()?;
    ensure!(
        y.len() == rows.len(),
        "model returned {} values for {} rows",
        y.len(),
        rows.len()
    );
    Ok(y.to_vec())
}

/// Landing time (seconds from the 40 nm ring to touchdown) for rows built by
/// `holdshort_twin::features::eta_rows`.
pub struct EtaModel {
    session: Session,
}

impl EtaModel {
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        let meta = Meta::load(dir)?;
        if meta.eta_features != ETA_FEATURES {
            bail!(
                "meta.json eta feature list {:?} does not match the twin's ETA_FEATURES {:?}",
                meta.eta_features,
                ETA_FEATURES
            );
        }
        Ok(Self {
            session: session(&dir.join("eta.onnx"))?,
        })
    }

    pub fn predict(&mut self, rows: &[[f32; ETA_N]]) -> anyhow::Result<Vec<f32>> {
        run(&mut self.session, rows)
    }
}

/// Arrivals in the hour `horizon` hours ahead (1–3) for rows built by `hourly_rows`.
pub struct HourlyModel {
    session: Session,
}

impl HourlyModel {
    pub fn load(dir: &Path, horizon: u8) -> anyhow::Result<Self> {
        ensure!((1..=3).contains(&horizon), "horizon must be 1, 2 or 3");
        let meta = Meta::load(dir)?;
        if meta.hourly_features != HOURLY_FEATURES {
            bail!("meta.json hourly feature list does not match HOURLY_FEATURES");
        }
        Ok(Self {
            session: session(&dir.join(format!("hourly_h{horizon}.onnx")))?,
        })
    }

    pub fn predict(&mut self, rows: &[[f32; HOURLY_N]]) -> anyhow::Result<Vec<f32>> {
        run(&mut self.session, rows)
    }
}

/// Feature rows for the 24 target hours of `target` (same rules as hourly.py): the
/// forecast for hour H is issued at H − horizon + 1, lags are the three hours
/// complete by then, and any hour on a day missing from `days` is NaN.
pub fn hourly_rows(
    days: &[(NaiveDate, [u32; 24])],
    target: NaiveDate,
    horizon: u8,
    profile: &[f32; 24],
) -> Vec<[f32; HOURLY_N]> {
    let count = |t: NaiveDateTime| -> f32 {
        days.iter()
            .find(|(d, _)| *d == t.date())
            .map_or(f32::NAN, |(_, c)| c[t.hour() as usize] as f32)
    };
    let midnight = target.and_hms_opt(0, 0, 0).expect("midnight");
    (0..24)
        .map(|h| {
            let ts = midnight + Duration::hours(h);
            let issue = ts - Duration::hours(horizon as i64 - 1);
            let frac = h as f64 / 24.0;
            [
                (TAU * frac).sin() as f32,
                (TAU * frac).cos() as f32,
                target.weekday().num_days_from_monday() as f32,
                count(issue - Duration::hours(1)),
                count(issue - Duration::hours(2)),
                count(issue - Duration::hours(3)),
                count(ts - Duration::hours(24)),
                count(ts - Duration::hours(24 * 7)),
                profile[h as usize],
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use holdshort_twin::features::ETA_N;
    use std::path::PathBuf;

    fn models_dir() -> PathBuf {
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../models"))
    }

    fn parity(name: &str) -> serde_json::Value {
        let p = models_dir().join("parity").join(name);
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    fn rows<const N: usize>(v: &serde_json::Value) -> Vec<[f32; N]> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|r| {
                let mut out = [0f32; N];
                for (i, x) in r.as_array().unwrap().iter().enumerate() {
                    out[i] = x.as_f64().map_or(f32::NAN, |x| x as f32);
                }
                out
            })
            .collect()
    }

    fn floats(v: &serde_json::Value) -> Vec<f32> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_f64().unwrap() as f32)
            .collect()
    }

    #[test]
    fn eta_parity() {
        let p = parity("eta.json");
        let x: Vec<[f32; ETA_N]> = rows(&p["rows"]);
        let got = EtaModel::load(&models_dir()).unwrap().predict(&x).unwrap();
        for (g, e) in got.iter().zip(floats(&p["expected"])) {
            assert!((g - e).abs() < 0.01, "{g} vs {e}");
        }
        assert_eq!(got.len(), x.len());
    }

    #[test]
    fn hourly_parity() {
        let p = parity("hourly.json");
        for h in 1..=3u8 {
            let x: Vec<[f32; HOURLY_N]> = rows(&p["rows"][h.to_string()]);
            let got = HourlyModel::load(&models_dir(), h)
                .unwrap()
                .predict(&x)
                .unwrap();
            for (g, e) in got.iter().zip(floats(&p["expected"][h.to_string()])) {
                assert!((g - e).abs() < 1e-3, "h={h}: {g} vs {e}");
            }
        }
    }

    /// The days in the parity file that have data, with their hourly counts.
    fn parity_days(p: &serde_json::Value) -> Vec<(NaiveDate, [u32; 24])> {
        let present: Vec<&str> = p["present"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d.as_str().unwrap())
            .collect();
        p["days"]
            .as_array()
            .unwrap()
            .iter()
            .zip(p["counts"].as_array().unwrap())
            .filter(|(d, _)| present.contains(&d.as_str().unwrap()))
            .map(|(d, c)| {
                let mut counts = [0u32; 24];
                for (i, v) in c.as_array().unwrap().iter().enumerate() {
                    counts[i] = v.as_u64().unwrap() as u32;
                }
                (
                    NaiveDate::parse_from_str(d.as_str().unwrap(), "%Y-%m-%d").unwrap(),
                    counts,
                )
            })
            .collect()
    }

    #[test]
    fn hourly_rows_match_python() {
        let p = parity("hourly.json");
        let days = parity_days(&p);
        let target = days.last().unwrap().0;
        let meta = Meta::load(&models_dir()).unwrap();
        for h in 1..=3u8 {
            let want: Vec<[f32; HOURLY_N]> = rows(&p["rows"][h.to_string()]);
            let got = hourly_rows(&days, target, h, &meta.profile);
            assert_eq!(got.len(), 24);
            for (g, w) in got.iter().zip(&want) {
                for (a, b) in g.iter().zip(w) {
                    assert!(
                        (a.is_nan() && b.is_nan()) || (a - b).abs() < 1e-5,
                        "h={h}: {g:?} vs {w:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn hourly_rows_missing_previous_day_is_nan() {
        let d = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        let days = vec![(d("2026-09-01"), [10; 24]), (d("2026-09-03"), [20; 24])];
        let profile = [5.0; 24];
        let r = hourly_rows(&days, d("2026-09-03"), 1, &profile);
        // 00:00 on the 3rd: the previous hours and the same hour yesterday are on the missing 2nd.
        assert!(r[0][3].is_nan() && r[0][4].is_nan() && r[0][5].is_nan());
        assert!(r[0][6].is_nan());
        // 05:00 with h = 1: lags are 04, 03, 02 on the 3rd.
        assert_eq!(&r[5][3..6], &[20.0, 20.0, 20.0]);
        // 2026-09-03 is a Thursday.
        assert_eq!(r[5][2], 3.0);
        assert_eq!(r[5][8], 5.0);
    }

    #[test]
    fn eta_load_rejects_feature_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        for f in ["eta.onnx", "meta.json"] {
            std::fs::copy(models_dir().join(f), dir.path().join(f)).unwrap();
        }
        let meta = std::fs::read_to_string(dir.path().join("meta.json")).unwrap();
        std::fs::write(
            dir.path().join("meta.json"),
            meta.replacen("\"brg_sin\"", "\"bearing\"", 1),
        )
        .unwrap();
        let err = EtaModel::load(dir.path()).err().expect("must fail");
        assert!(err.to_string().contains("feature"), "{err}");
    }
}
