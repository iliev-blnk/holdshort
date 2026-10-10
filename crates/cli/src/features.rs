//! `holdshort features`: the forecasting models' input tables, one day at a time.

use std::sync::Arc;

use arrow::array::{
    ArrayRef, Float32Array, Float64Array, Int64Array, StringArray, UInt8Array, UInt32Array,
};
use chrono::NaiveDate;
use holdshort_core::airport::Airport;
use holdshort_twin::features::{ETA_FEATURES, EtaRow, eta_rows, hourly_counts};
use holdshort_twin::height::Weather;

use crate::parquet_io::write;
use crate::store::Store;
use crate::twin::compute_day;

fn eta_parquet(date: &str, rows: &[EtaRow]) -> anyhow::Result<Vec<u8>> {
    let mut cols: Vec<(&str, ArrayRef)> = vec![
        ("date", Arc::new(StringArray::from(vec![date; rows.len()]))),
        (
            "flight_id",
            Arc::new(
                rows.iter()
                    .map(|r| Some(r.flight_id.as_str()))
                    .collect::<StringArray>(),
            ),
        ),
        (
            "t0_ms",
            Arc::new(rows.iter().map(|r| r.t0_ms).collect::<Int64Array>()),
        ),
        (
            "arr_rwy",
            Arc::new(
                rows.iter()
                    .map(|r| r.arr_rwy.as_deref())
                    .collect::<StringArray>(),
            ),
        ),
        (
            "sector",
            Arc::new(rows.iter().map(|r| r.sector).collect::<UInt8Array>()),
        ),
        (
            "dist_nm",
            Arc::new(rows.iter().map(|r| r.dist_nm).collect::<Float64Array>()),
        ),
        (
            "asma_s",
            Arc::new(rows.iter().map(|r| r.asma_s).collect::<Float64Array>()),
        ),
    ];
    for (i, name) in ETA_FEATURES.iter().enumerate() {
        cols.push((
            name,
            Arc::new(rows.iter().map(|r| r.x[i]).collect::<Float32Array>()),
        ));
    }
    write(cols)
}

fn hourly_parquet(date: &str, counts: &[u32; 24]) -> anyhow::Result<Vec<u8>> {
    write(vec![
        (
            "date",
            Arc::new(StringArray::from(vec![date; 24])) as ArrayRef,
        ),
        ("hour", Arc::new((0..24u8).collect::<UInt8Array>())),
        (
            "arrivals",
            Arc::new(counts.iter().copied().collect::<UInt32Array>()),
        ),
    ])
}

/// Compute and store one day's feature tables; returns a one-line summary.
pub fn run_day(store: &Store, ap: &'static Airport, day: NaiveDate) -> anyhow::Result<String> {
    let (twin, obs, metars) = compute_day(store, ap, day)?;
    let start = day
        .and_hms_opt(0, 0, 0)
        .expect("midnight")
        .and_utc()
        .timestamp_millis();
    let rows = eta_rows(ap, &twin, &obs, &Weather::new(&metars));
    let counts = hourly_counts(&twin, start);
    let date = day.to_string();
    let dir = format!("{}/features/date={date}", ap.icao);
    store.put(&format!("{dir}/eta.parquet"), eta_parquet(&date, &rows)?)?;
    store.put(
        &format!("{dir}/hourly.parquet"),
        hourly_parquet(&date, &counts)?,
    )?;
    Ok(format!(
        "{} eta rows, {} arrivals in 24 hours",
        rows.len(),
        counts.iter().sum::<u32>()
    ))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::compact::obs_key;
    use crate::parquet_io::obs_parquet;
    use crate::store::Store;
    use arrow::array::{Array, AsArray};
    use arrow::datatypes::UInt32Type;
    use chrono::NaiveDate;
    use holdshort_core::airport::LTFM;
    use holdshort_core::{Obs, Src};
    use holdshort_twin::features::ETA_FEATURES;
    use holdshort_twin::geo::{NM_M, from_runway};
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    pub(crate) fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 8).unwrap()
    }

    /// A straight-in 3° approach to 35L from 45 nm (crosses the 40 nm ring), 5 s steps, from 01:00.
    pub(crate) fn long_approach() -> Vec<Obs> {
        let r = LTFM.runways.iter().find(|r| r.name == "35L").unwrap();
        let t0 = 1_791_417_600_000 + 3_600_000;
        let step = 140.0 * NM_M / 3600.0 * 5.0;
        let n = (45.0 * NM_M / step) as i64;
        (0..=n)
            .map(|i| {
                let along = -45.0 * NM_M + i as f64 * step;
                let (lat, lon) = from_runway(r, along.min(-0.3 * NM_M), 0.0);
                let alt =
                    r.elevation_ft + (-along.min(-0.3 * NM_M) * 3f64.to_radians().tan()) / 0.3048;
                Obs {
                    ts_ms: t0 + i * 5_000,
                    hex: "aaaaaa".into(),
                    flight: Some("THY1".into()),
                    reg: None,
                    ac_type: None,
                    lat,
                    lon,
                    alt_baro_ft: Some(alt as i32),
                    alt_geom_ft: Some(alt as i32),
                    on_ground: false,
                    gs_kt: Some(140.0),
                    track_deg: Some(r.heading_deg as f32),
                    vrate_fpm: Some(-740),
                    squawk: None,
                    category: None,
                    src: Src::History,
                }
            })
            .filter(|o| o.ts_ms <= t0 + ((45.0 - 0.3) * NM_M / step) as i64 * 5_000)
            .collect()
    }

    fn read(store: &Store, key: &str) -> arrow::array::RecordBatch {
        let bytes = bytes::Bytes::from(store.get(key).unwrap());
        let mut r = ParquetRecordBatchReaderBuilder::try_new(bytes)
            .unwrap()
            .build()
            .unwrap();
        r.next().unwrap().unwrap()
    }

    #[test]
    fn features_day_writes_both_files() {
        let store = Store::memory();
        store
            .put(
                &obs_key(&LTFM, day()),
                obs_parquet(&long_approach()).unwrap(),
            )
            .unwrap();
        let summary = run_day(&store, &LTFM, day()).unwrap();
        assert_eq!(summary, "1 eta rows, 1 arrivals in 24 hours");

        let eta = read(&store, "LTFM/features/date=2026-10-08/eta.parquet");
        assert_eq!(eta.num_rows(), 1);
        let names: Vec<String> = eta
            .schema()
            .fields()
            .iter()
            .map(|f| f.name().clone())
            .collect();
        assert_eq!(
            &names[..7],
            [
                "date",
                "flight_id",
                "t0_ms",
                "arr_rwy",
                "sector",
                "dist_nm",
                "asma_s"
            ]
        );
        assert_eq!(&names[7..], ETA_FEATURES);
        assert!(
            eta.column(6)
                .as_primitive::<arrow::datatypes::Float64Type>()
                .value(0)
                > 900.0
        );

        let hourly = read(&store, "LTFM/features/date=2026-10-08/hourly.parquet");
        assert_eq!(hourly.num_rows(), 24);
        let arrivals = hourly
            .column_by_name("arrivals")
            .unwrap()
            .as_primitive::<UInt32Type>();
        assert_eq!(arrivals.value(1), 1);
        assert_eq!(arrivals.iter().flatten().sum::<u32>(), 1);
        assert_eq!(hourly.column_by_name("date").unwrap().len(), 24);
    }
}
