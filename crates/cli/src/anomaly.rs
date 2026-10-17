//! `holdshort anomaly-check`: score days of approaches with the kNN detector and
//! measure it against the twin's holds and go-arounds (research/ANOMALY.md).

use std::path::Path;

use chrono::NaiveDate;
use holdshort_core::airport::Airport;
use holdshort_models::approach::{ApproachModel, average_precision, recall_at};
use holdshort_twin::approach::approach_rows;
use holdshort_twin::height::Weather;

use crate::store::Store;
use crate::twin::compute_day;

pub struct AnomalyCheck {
    /// Scored approaches.
    pub n: usize,
    /// Scored approaches with a hold or a go-around.
    pub n_labels: usize,
    pub ap: f64,
    pub recall_1pct: f64,
    pub flagged: usize,
}

pub fn check(
    store: &Store,
    ap: &'static Airport,
    days: &[NaiveDate],
    models: &Path,
) -> anyhow::Result<AnomalyCheck> {
    let model = ApproachModel::load(models)?;
    let (mut labels, mut scores, mut flagged) = (Vec::new(), Vec::new(), 0);
    for &day in days {
        let (twin, obs, metars) = compute_day(store, ap, day)?;
        for row in approach_rows(ap, &twin, &obs, &Weather::new(&metars)) {
            if let Some(s) = model.score(&row) {
                labels.push(row.label_ga || row.label_hold);
                scores.push(s.score);
                flagged += s.flag as usize;
            }
        }
    }
    Ok(AnomalyCheck {
        n: scores.len(),
        n_labels: labels.iter().filter(|&&l| l).count(),
        ap: average_precision(&labels, &scores),
        recall_1pct: recall_at(&labels, &scores, 0.01),
        flagged,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compact::obs_key;
    use crate::features::tests::{day, long_approach_to};
    use crate::parquet_io::obs_parquet;
    use holdshort_core::airport::LTFM;
    use std::path::PathBuf;

    fn models_dir() -> PathBuf {
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../models"))
    }

    #[test]
    fn anomaly_check_on_fixture_day() {
        let store = Store::memory();
        store
            .put(
                &obs_key(&LTFM, day()),
                obs_parquet(&long_approach_to("35R")).unwrap(),
            )
            .unwrap();
        let r = check(&store, &LTFM, &[day()], &models_dir()).unwrap();
        assert_eq!(r.n, 1);
        assert_eq!(r.n_labels, 0);
        // No labels: AP and recall are undefined.
        assert!(r.ap.is_nan() && r.recall_1pct.is_nan());
        assert!(r.flagged <= 1);
    }

    #[test]
    fn days_without_data_are_an_error() {
        assert!(check(&Store::memory(), &LTFM, &[day()], &models_dir()).is_err());
    }
}
