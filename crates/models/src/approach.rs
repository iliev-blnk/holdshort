//! The approach anomaly detector (sub-project 5): kNN distance to the normal
//! approaches of the same runway, in pure Rust. Trained and calibrated in
//! `research/holdshort_research/approach.py`, which writes `approach.json` and
//! `approach_ref.f32`.

use std::path::Path;

use anyhow::{Context, ensure};
use holdshort_twin::approach::{ApproachRow, METRICS, PATH_N};
use holdshort_twin::geo::{NM_M, RunwayEnd, from_runway};
use serde::Deserialize;

/// A metric far from the reference: its value and the reference median.
#[derive(Debug, Clone, PartialEq)]
pub struct Reason {
    pub metric: &'static str,
    pub value: f32,
    pub typical: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApproachScore {
    /// Mean path distance to the k nearest reference approaches.
    pub score: f64,
    /// Percentile among the validation week's approaches to the same runway.
    pub pct: f64,
    pub flag: bool,
    pub reasons: Vec<Reason>,
    /// Index of the nearest reference approach within its runway.
    pub nearest: usize,
}

/// Points per path and values per point.
const M: usize = PATH_N / 3;
const REASON_Z: f64 = 3.0;
const MAX_REASONS: usize = 2;

#[derive(Deserialize)]
struct Meta {
    m: usize,
    k: usize,
    metrics: Vec<String>,
    runways: Vec<RunwayMeta>,
}

#[derive(Deserialize)]
struct RunwayMeta {
    name: String,
    n: usize,
    ids: Vec<String>,
    median: Vec<Option<f64>>,
    mad: Vec<Option<f64>>,
    quantiles: Vec<f64>,
    threshold: f64,
}

struct RunwayRef {
    meta: RunwayMeta,
    /// n × PATH_N values, row-major.
    paths: Vec<f32>,
}

pub struct ApproachModel {
    k: usize,
    runways: Vec<RunwayRef>,
}

impl ApproachModel {
    /// Read `approach.json` and `approach_ref.f32` from `dir`.
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        let meta: Meta = serde_json::from_str(
            &std::fs::read_to_string(dir.join("approach.json")).context("approach.json")?,
        )?;
        ensure!(
            meta.m == M,
            "approach.json has m = {}, expected {M}",
            meta.m
        );
        ensure!(
            meta.metrics == METRICS,
            "approach.json metrics differ from the twin's"
        );
        let bytes = std::fs::read(dir.join("approach_ref.f32")).context("approach_ref.f32")?;
        let total: usize = meta.runways.iter().map(|r| r.n).sum();
        ensure!(
            bytes.len() == total * PATH_N * 4,
            "approach_ref.f32 has {} bytes, expected {}",
            bytes.len(),
            total * PATH_N * 4
        );
        let mut values = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c));
        let mut runways = Vec::new();
        for r in meta.runways {
            ensure!(
                r.quantiles.len() == 101,
                "{}: {} quantiles",
                r.name,
                r.quantiles.len()
            );
            ensure!(
                r.ids.len() == r.n && r.median.len() == 7 && r.mad.len() == 7,
                "{}: bad sizes",
                r.name
            );
            let paths = values.by_ref().take(r.n * PATH_N).collect();
            runways.push(RunwayRef { meta: r, paths });
        }
        Ok(Self { k: meta.k, runways })
    }

    fn runway(&self, name: &str) -> Option<&RunwayRef> {
        self.runways.iter().find(|r| r.meta.name == name)
    }

    /// None when the row is unscored or its runway has no reference.
    pub fn score(&self, row: &ApproachRow) -> Option<ApproachScore> {
        if row.unscored.is_some() {
            return None;
        }
        let r = self.runway(row.runway.as_deref()?)?;
        let mut d: Vec<(f64, usize)> = r
            .paths
            .as_chunks::<PATH_N>()
            .0
            .iter()
            .enumerate()
            .map(|(i, p)| (distance(&row.path, p), i))
            .collect();
        let nearest = d
            .iter()
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))?
            .1;
        let k = self.k.min(d.len());
        d.select_nth_unstable_by(k - 1, |a, b| a.0.total_cmp(&b.0));
        let mut best: Vec<f64> = d[..k].iter().map(|x| x.0).collect();
        best.sort_by(f64::total_cmp);
        let score = best.iter().sum::<f64>() / k as f64;
        Some(ApproachScore {
            score,
            pct: percentile(score, &r.meta.quantiles),
            flag: score >= r.meta.threshold,
            reasons: reasons(&row.metrics, &r.meta),
            nearest,
        })
    }

    /// The nearest reference approach as 32 `[lon, lat]` points.
    pub fn nearest_path(&self, rwy: &RunwayEnd, runway: &str, idx: usize) -> Vec<[f64; 2]> {
        let Some(r) = self.runway(runway) else {
            return Vec::new();
        };
        r.paths[idx * PATH_N..(idx + 1) * PATH_N]
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| {
                let (lat, lon) = from_runway(rwy, p[0] as f64 * NM_M, p[1] as f64 * NM_M);
                [lon, lat]
            })
            .collect()
    }

    /// `date/flight_id` of a reference approach.
    pub fn nearest_id(&self, runway: &str, idx: usize) -> &str {
        self.runway(runway).map_or("", |r| r.meta.ids[idx].as_str())
    }
}

/// Mean over the points of the Euclidean distance in (x, y, h).
fn distance(a: &[f32; PATH_N], b: &[f32; PATH_N]) -> f64 {
    a.as_chunks::<3>()
        .0
        .iter()
        .zip(b.as_chunks::<3>().0)
        .map(|(p, q)| {
            (0..3)
                .map(|j| (p[j] as f64 - q[j] as f64).powi(2))
                .sum::<f64>()
                .sqrt()
        })
        .sum::<f64>()
        / M as f64
}

/// Position of `s` in a 101-value quantile table, 0–100.
fn percentile(s: f64, q: &[f64]) -> f64 {
    if s <= q[0] {
        return 0.0;
    }
    if s >= q[100] {
        return 100.0;
    }
    let j = q.partition_point(|&v| v <= s) - 1;
    let gap = q[j + 1] - q[j];
    j as f64 + if gap > 0.0 { (s - q[j]) / gap } else { 0.0 }
}

fn reasons(metrics: &[f32; 7], r: &RunwayMeta) -> Vec<Reason> {
    let mut z: Vec<(f64, usize)> = (0..7)
        .filter_map(|i| {
            let (med, mad) = (r.median[i]?, r.mad[i]?);
            let v = metrics[i] as f64;
            (mad > 0.0 && v.is_finite()).then(|| (((v - med) / mad).abs(), i))
        })
        .filter(|(z, _)| *z >= REASON_Z)
        .collect();
    // Largest first; equal |z| keep the metric order (as Python's stable sort).
    z.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    z.into_iter()
        .take(MAX_REASONS)
        .map(|(_, i)| Reason {
            metric: METRICS[i],
            value: metrics[i],
            typical: r.median[i].unwrap_or(f64::NAN) as f32,
        })
        .collect()
}

/// Positives in score order (highest first, ties in input order); NaN without positives.
fn ranked(labels: &[bool], scores: &[f64]) -> Vec<bool> {
    let mut idx: Vec<usize> = (0..scores.len()).collect();
    idx.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]));
    idx.into_iter().map(|i| labels[i]).collect()
}

/// Mean over the positives of the precision at their rank.
pub fn average_precision(labels: &[bool], scores: &[f64]) -> f64 {
    let (mut hits, mut sum) = (0usize, 0.0);
    for (rank, hit) in ranked(labels, scores).into_iter().enumerate() {
        if hit {
            hits += 1;
            sum += hits as f64 / (rank + 1) as f64;
        }
    }
    if hits == 0 {
        f64::NAN
    } else {
        sum / hits as f64
    }
}

/// Share of the positives among the top ⌈rate·N⌉ scores.
pub fn recall_at(labels: &[bool], scores: &[f64], rate: f64) -> f64 {
    let total = labels.iter().filter(|&&l| l).count();
    if total == 0 {
        return f64::NAN;
    }
    let n = (rate * labels.len() as f64).ceil() as usize;
    let top = ranked(labels, scores)
        .into_iter()
        .take(n)
        .filter(|&h| h)
        .count();
    top as f64 / total as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use holdshort_core::airport::LTFM;
    use holdshort_core::area::distance_nm;
    use std::path::PathBuf;

    fn models_dir() -> PathBuf {
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../models"))
    }

    fn f(v: &serde_json::Value) -> f32 {
        v.as_f64().map_or(f32::NAN, |x| x as f32)
    }

    fn row_from(v: &serde_json::Value) -> ApproachRow {
        let mut metrics = [f32::NAN; 7];
        for (i, x) in v["metrics"].as_array().unwrap().iter().enumerate() {
            metrics[i] = f(x);
        }
        let mut path = [0f32; PATH_N];
        for (i, x) in v["path"].as_array().unwrap().iter().enumerate() {
            path[i] = f(x);
        }
        ApproachRow {
            flight_id: "p".into(),
            touchdown_ms: 0,
            runway: Some(v["runway"].as_str().unwrap().into()),
            unscored: None,
            label_ga: false,
            label_hold: false,
            metrics,
            path,
        }
    }

    #[test]
    fn approach_parity() {
        let m = ApproachModel::load(&models_dir()).unwrap();
        let p: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(models_dir().join("parity/approach.json")).unwrap(),
        )
        .unwrap();
        let rows = p["rows"].as_array().unwrap();
        assert!(rows.len() >= 60, "{} parity rows", rows.len());
        assert!(rows.iter().any(|r| r["flag"].as_bool().unwrap()));
        for (i, r) in rows.iter().enumerate() {
            let s = m.score(&row_from(r)).unwrap();
            let want = r["score"].as_f64().unwrap();
            assert!(
                (s.score - want).abs() < 1e-4,
                "row {i}: {} vs {want}",
                s.score
            );
            assert!(
                (s.pct - r["pct"].as_f64().unwrap()).abs() < 1e-3,
                "row {i} pct"
            );
            assert_eq!(s.flag, r["flag"].as_bool().unwrap(), "row {i} flag");
            assert_eq!(
                s.nearest as u64,
                r["nearest"].as_u64().unwrap(),
                "row {i} nearest"
            );
            let keys: Vec<&str> = s.reasons.iter().map(|r| r.metric).collect();
            let want_keys: Vec<&str> = r["reasons"]
                .as_array()
                .unwrap()
                .iter()
                .map(|k| k.as_str().unwrap())
                .collect();
            assert_eq!(keys, want_keys, "row {i} reasons");
        }
    }

    #[test]
    fn unknown_runway_is_unscored() {
        let m = ApproachModel::load(&models_dir()).unwrap();
        let mut row = ApproachRow {
            flight_id: "x".into(),
            touchdown_ms: 0,
            runway: Some("36".into()),
            unscored: None,
            label_ga: false,
            label_hold: false,
            metrics: [0.0; 7],
            path: [0.0; PATH_N],
        };
        assert_eq!(m.score(&row), None);
        row.runway = Some("35R".into());
        assert!(m.score(&row).is_some());
        row.unscored = Some("coverage");
        assert_eq!(m.score(&row), None);
    }

    /// A one-runway model with two straight reference paths and metric 0 at zero MAD.
    fn write_model(dir: &Path, n_ref_values: usize) {
        let json = serde_json::json!({
            "m": 32, "k": 1, "h_scale": 1000, "metrics": METRICS,
            "runways": [{
                "name": "35R", "n": 2, "ids": ["d/a", "d/b"],
                "median": [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                "mad": [0.0, 1.0, 1.0, 1.0, 1.0, 1.0, null],
                "quantiles": (0..=100).map(|i| i as f64 / 10.0).collect::<Vec<_>>(),
                "threshold": 9.9,
            }]
        });
        std::fs::write(dir.join("approach.json"), json.to_string()).unwrap();
        let mut bytes = Vec::new();
        for r in 0..2 {
            for i in 0..32 {
                for v in [-40.0 + 40.0 * i as f32 / 31.0, r as f32, 0.0] {
                    bytes.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
        bytes.truncate(n_ref_values * 4);
        std::fs::write(dir.join("approach_ref.f32"), bytes).unwrap();
    }

    #[test]
    fn synthetic_model_scores_and_explains() {
        let tmp = tempfile::tempdir().unwrap();
        write_model(tmp.path(), 2 * PATH_N);
        let m = ApproachModel::load(tmp.path()).unwrap();
        let mut row = ApproachRow {
            flight_id: "q".into(),
            touchdown_ms: 0,
            runway: Some("35R".into()),
            unscored: None,
            label_ga: false,
            label_hold: false,
            metrics: [50.0, 0.5, 4.0, -3.5, 0.0, 0.0, 99.0],
            path: [0.0; PATH_N],
        };
        for i in 0..32 {
            row.path[3 * i] = -40.0 + 40.0 * i as f32 / 31.0;
            row.path[3 * i + 1] = 0.75;
        }
        let s = m.score(&row).unwrap();
        // Nearest is the path at y = 1, 0.25 nm away; k = 1.
        assert!((s.score - 0.25).abs() < 1e-6, "{}", s.score);
        assert_eq!(s.nearest, 1);
        assert_eq!(m.nearest_id("35R", 1), "d/b");
        assert!((s.pct - 2.5).abs() < 1e-9, "{}", s.pct);
        assert!(!s.flag);
        // Metric 0 has zero MAD and metric 6 no MAD: neither is a reason.
        let keys: Vec<&str> = s.reasons.iter().map(|r| r.metric).collect();
        assert_eq!(keys, ["turn_deg", "level_min"]);
        assert_eq!(s.reasons[0].value, 4.0);
        assert_eq!(s.reasons[0].typical, 0.0);

        for i in 0..32 {
            row.path[3 * i + 1] = 30.0;
        }
        let far = m.score(&row).unwrap();
        assert_eq!(far.pct, 100.0);
        assert!(far.flag);
    }

    #[test]
    fn load_rejects_truncated_ref() {
        let tmp = tempfile::tempdir().unwrap();
        write_model(tmp.path(), 2 * PATH_N - 1);
        assert!(ApproachModel::load(tmp.path()).is_err());
    }

    #[test]
    fn nearest_path_is_32_points_near_ltfm() {
        let m = ApproachModel::load(&models_dir()).unwrap();
        let r = LTFM.runways.iter().find(|r| r.name == "34L").unwrap();
        let p = m.nearest_path(r, "34L", 0);
        assert_eq!(p.len(), 32);
        let [lon, lat] = *p.last().unwrap();
        assert!(distance_nm(lat, lon, r.lat, r.lon) < 0.01);
        assert!(
            p.iter()
                .all(|[lon, lat]| distance_nm(*lat, *lon, LTFM.lat, LTFM.lon) < 60.0)
        );
    }

    #[test]
    fn ap_and_recall_match_python_example() {
        let labels = [true, false, true, false];
        let scores = [0.9, 0.8, 0.7, 0.1];
        assert!((average_precision(&labels, &scores) - (1.0 + 2.0 / 3.0) / 2.0).abs() < 1e-12);
        assert!((recall_at(&labels, &scores, 0.25) - 0.5).abs() < 1e-12);
        assert!((recall_at(&labels, &scores, 0.75) - 1.0).abs() < 1e-12);
        assert!(average_precision(&[false; 3], &[1.0; 3]).is_nan());
        assert!(recall_at(&[false; 3], &[1.0; 3], 0.01).is_nan());
    }
}
