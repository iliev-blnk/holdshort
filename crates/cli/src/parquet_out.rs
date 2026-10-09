//! Obs and poll rows as zstd Parquet, one Arrow column per field.

use std::sync::Arc;

use arrow::array::{
    ArrayRef, BooleanArray, Float32Array, Float64Array, Int32Array, Int64Array, RecordBatch,
    StringArray, UInt32Array,
};
use holdshort_core::{Obs, Poll, Src};
use parquet::arrow::ArrowWriter;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::WriterProperties;

fn src_name(s: Src) -> &'static str {
    match s {
        Src::Adsblol => "adsblol",
        Src::Opensky => "opensky",
        Src::History => "history",
    }
}

fn write(columns: Vec<(&str, ArrayRef)>) -> anyhow::Result<Vec<u8>> {
    let batch = RecordBatch::try_from_iter(columns)?;
    let props = WriterProperties::builder()
        .set_compression(Compression::ZSTD(ZstdLevel::try_new(3)?))
        .build();
    let mut out = Vec::new();
    let mut w = ArrowWriter::try_new(&mut out, batch.schema(), Some(props))?;
    w.write(&batch)?;
    w.close()?;
    Ok(out)
}

// Arrow stores columns, not rows: build each column with one pass over the slice.
pub fn obs_parquet(obs: &[Obs]) -> anyhow::Result<Vec<u8>> {
    let text = |f: fn(&Obs) -> Option<&str>| -> ArrayRef {
        Arc::new(obs.iter().map(f).collect::<StringArray>())
    };
    write(vec![
        (
            "ts_ms",
            Arc::new(obs.iter().map(|o| o.ts_ms).collect::<Int64Array>()) as ArrayRef,
        ),
        (
            "hex",
            Arc::new(
                obs.iter()
                    .map(|o| Some(o.hex.as_str()))
                    .collect::<StringArray>(),
            ),
        ),
        ("flight", text(|o| o.flight.as_deref())),
        ("reg", text(|o| o.reg.as_deref())),
        ("ac_type", text(|o| o.ac_type.as_deref())),
        (
            "lat",
            Arc::new(obs.iter().map(|o| o.lat).collect::<Float64Array>()),
        ),
        (
            "lon",
            Arc::new(obs.iter().map(|o| o.lon).collect::<Float64Array>()),
        ),
        (
            "alt_baro_ft",
            Arc::new(obs.iter().map(|o| o.alt_baro_ft).collect::<Int32Array>()),
        ),
        (
            "alt_geom_ft",
            Arc::new(obs.iter().map(|o| o.alt_geom_ft).collect::<Int32Array>()),
        ),
        (
            "on_ground",
            Arc::new(
                obs.iter()
                    .map(|o| Some(o.on_ground))
                    .collect::<BooleanArray>(),
            ),
        ),
        (
            "gs_kt",
            Arc::new(obs.iter().map(|o| o.gs_kt).collect::<Float32Array>()),
        ),
        (
            "track_deg",
            Arc::new(obs.iter().map(|o| o.track_deg).collect::<Float32Array>()),
        ),
        (
            "vrate_fpm",
            Arc::new(obs.iter().map(|o| o.vrate_fpm).collect::<Int32Array>()),
        ),
        ("squawk", text(|o| o.squawk.as_deref())),
        ("category", text(|o| o.category.as_deref())),
        (
            "src",
            Arc::new(
                obs.iter()
                    .map(|o| Some(src_name(o.src)))
                    .collect::<StringArray>(),
            ),
        ),
    ])
}

pub fn polls_parquet(polls: &[Poll]) -> anyhow::Result<Vec<u8>> {
    write(vec![
        (
            "ts_ms",
            Arc::new(polls.iter().map(|p| p.ts_ms).collect::<Int64Array>()) as ArrayRef,
        ),
        (
            "src",
            Arc::new(
                polls
                    .iter()
                    .map(|p| Some(src_name(p.src)))
                    .collect::<StringArray>(),
            ),
        ),
        (
            "ok",
            Arc::new(polls.iter().map(|p| Some(p.ok)).collect::<BooleanArray>()),
        ),
        (
            "count",
            Arc::new(polls.iter().map(|p| p.count).collect::<UInt32Array>()),
        ),
        (
            "latency_ms",
            Arc::new(polls.iter().map(|p| p.latency_ms).collect::<UInt32Array>()),
        ),
        (
            "error",
            Arc::new(
                polls
                    .iter()
                    .map(|p| p.error.as_deref())
                    .collect::<StringArray>(),
            ),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use holdshort_core::Src;
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    fn obs(flight: Option<&str>) -> Obs {
        Obs {
            ts_ms: 1,
            hex: "aaaaaa".into(),
            flight: flight.map(str::to_string),
            reg: None,
            ac_type: Some("A21N".into()),
            lat: 41.0,
            lon: 28.0,
            alt_baro_ft: Some(3000),
            alt_geom_ft: None,
            on_ground: false,
            gs_kt: Some(150.5),
            track_deg: None,
            vrate_fpm: Some(-640),
            squawk: None,
            category: None,
            src: Src::History,
        }
    }

    #[test]
    fn parquet_round_trip() {
        let bytes = obs_parquet(&[obs(Some("THY1")), obs(None), obs(Some("THY2"))]).unwrap();
        let reader = ParquetRecordBatchReaderBuilder::try_new(bytes::Bytes::from(bytes))
            .unwrap()
            .build()
            .unwrap();
        let batches: Vec<_> = reader.map(Result::unwrap).collect();
        let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(rows, 3);
        let flight = batches[0].column_by_name("flight").unwrap();
        assert_eq!(flight.null_count(), 1);
        assert!(batches[0].column_by_name("src").is_some());
    }

    #[test]
    fn polls_parquet_writes() {
        let p = Poll {
            ts_ms: 1,
            src: Src::Opensky,
            ok: false,
            count: 0,
            latency_ms: 5000,
            error: Some("timeout".into()),
        };
        let bytes = polls_parquet(&[p]).unwrap();
        assert!(bytes.starts_with(b"PAR1"));
    }
}
