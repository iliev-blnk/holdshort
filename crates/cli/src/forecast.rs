//! `holdshort forecast-check`: run the ONNX models over days of feature tables and
//! compare them with their baselines (the numbers in research/REPORT.md).

use std::path::Path;

use anyhow::Context;
use arrow::array::{AsArray, RecordBatch};
use arrow::datatypes::{Float32Type, Float64Type, UInt32Type};
use chrono::{Duration, NaiveDate};
use holdshort_core::airport::Airport;
use holdshort_models::{EtaModel, HourlyModel, Meta, hourly_rows};
use holdshort_twin::features::{ETA_FEATURES, ETA_N};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use crate::store::Store;

pub struct EtaScore {
    pub n: usize,
    pub model_mae_s: f64,
    /// The global-median baseline (training median from meta.json).
    pub baseline_mae_s: f64,
}

pub struct HourlyScore {
    pub horizon: u8,
    pub n: usize,
    pub model_mae: f64,
    /// Persistence: the last completed hour (the profile where it is missing).
    pub baseline_mae: f64,
}

pub struct Report {
    pub eta: EtaScore,
    pub hourly: Vec<HourlyScore>,
}

fn table(
    store: &Store,
    ap: &Airport,
    day: NaiveDate,
    name: &str,
) -> anyhow::Result<Option<Vec<RecordBatch>>> {
    let key = format!("{}/features/date={day}/{name}.parquet", ap.icao);
    if !store.exists(&key)? {
        return Ok(None);
    }
    let bytes = bytes::Bytes::from(store.get(&key)?);
    let reader = ParquetRecordBatchReaderBuilder::try_new(bytes)?.build()?;
    Ok(Some(reader.collect::<Result<Vec<_>, _>>()?))
}

fn need(
    t: Option<Vec<RecordBatch>>,
    ap: &Airport,
    day: NaiveDate,
) -> anyhow::Result<Vec<RecordBatch>> {
    t.with_context(|| {
        format!(
            "no feature tables for {} {day}: run `holdshort features` first",
            ap.icao
        )
    })
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len().max(1) as f64
}

fn hourly_counts(batches: &[RecordBatch]) -> [u32; 24] {
    let mut c = [0u32; 24];
    for b in batches {
        let hours = b
            .column_by_name("hour")
            .expect("hour")
            .as_primitive::<arrow::datatypes::UInt8Type>();
        let arr = b
            .column_by_name("arrivals")
            .expect("arrivals")
            .as_primitive::<UInt32Type>();
        for i in 0..b.num_rows() {
            c[hours.value(i) as usize] = arr.value(i);
        }
    }
    c
}

pub fn check(
    store: &Store,
    ap: &Airport,
    days: &[NaiveDate],
    models: &Path,
) -> anyhow::Result<Report> {
    let meta = Meta::load(models)?;
    let mut eta_model = EtaModel::load(models)?;
    let (mut model_err, mut base_err) = (Vec::new(), Vec::new());
    for &day in days {
        for b in need(table(store, ap, day, "eta")?, ap, day)? {
            let y = b
                .column_by_name("asma_s")
                .expect("asma_s")
                .as_primitive::<Float64Type>();
            let cols: Vec<_> = ETA_FEATURES
                .iter()
                .map(|f| {
                    b.column_by_name(f)
                        .with_context(|| format!("column {f}"))
                        .map(|c| c.as_primitive::<Float32Type>().clone())
                })
                .collect::<anyhow::Result<_>>()?;
            let rows: Vec<[f32; ETA_N]> = (0..b.num_rows())
                .map(|i| std::array::from_fn(|j| cols[j].value(i)))
                .collect();
            for (i, p) in eta_model.predict(&rows)?.into_iter().enumerate() {
                model_err.push((p as f64 - y.value(i)).abs());
                base_err.push((meta.eta_global_median_s - y.value(i)).abs());
            }
        }
    }

    // Counts for the checked days plus up to a week before them, for the lags.
    let first = *days.iter().min().context("no days")?;
    let mut counts = Vec::new();
    let mut d = first - Duration::days(7);
    let last = *days.iter().max().expect("non-empty");
    while d <= last {
        let t = table(store, ap, d, "hourly")?;
        if days.contains(&d) {
            counts.push((d, hourly_counts(&need(t, ap, d)?)));
        } else if let Some(t) = t {
            counts.push((d, hourly_counts(&t)));
        }
        d += Duration::days(1);
    }
    let mut hourly = Vec::new();
    for h in 1..=3u8 {
        let mut model = HourlyModel::load(models, h)?;
        let (mut m_err, mut b_err) = (Vec::new(), Vec::new());
        for &day in days {
            let rows = hourly_rows(&counts, day, h, &meta.profile);
            let y = counts
                .iter()
                .find(|(d, _)| *d == day)
                .expect("checked day loaded")
                .1;
            for (hr, (p, r)) in model.predict(&rows)?.into_iter().zip(&rows).enumerate() {
                let persistence = if r[3].is_nan() { r[8] } else { r[3] };
                m_err.push((p as f64 - y[hr] as f64).abs());
                b_err.push((persistence as f64 - y[hr] as f64).abs());
            }
        }
        hourly.push(HourlyScore {
            horizon: h,
            n: m_err.len(),
            model_mae: mean(&m_err),
            baseline_mae: mean(&b_err),
        });
    }
    Ok(Report {
        eta: EtaScore {
            n: model_err.len(),
            model_mae_s: mean(&model_err),
            baseline_mae_s: mean(&base_err),
        },
        hourly,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compact::obs_key;
    use crate::features::tests::{day, long_approach};
    use crate::parquet_io::obs_parquet;
    use crate::store::Store;
    use holdshort_core::airport::LTFM;
    use std::path::PathBuf;

    fn models_dir() -> PathBuf {
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../models"))
    }

    #[test]
    fn forecast_check_on_fixture_day() {
        let store = Store::memory();
        store
            .put(
                &obs_key(&LTFM, day()),
                obs_parquet(&long_approach()).unwrap(),
            )
            .unwrap();
        crate::features::run_day(&store, &LTFM, day()).unwrap();
        let r = check(&store, &LTFM, &[day()], &models_dir()).unwrap();
        assert_eq!(r.eta.n, 1);
        assert!(r.eta.model_mae_s.is_finite() && r.eta.baseline_mae_s.is_finite());
        assert_eq!(r.hourly.len(), 3);
        for h in &r.hourly {
            assert_eq!(h.n, 24);
            assert!(h.model_mae.is_finite() && h.baseline_mae.is_finite());
        }
    }

    #[test]
    fn days_without_features_are_an_error() {
        let err = check(&Store::memory(), &LTFM, &[day()], &models_dir())
            .err()
            .unwrap();
        assert!(err.to_string().contains("holdshort features"), "{err}");
    }
}
