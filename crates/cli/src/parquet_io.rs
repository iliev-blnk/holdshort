//! Parquet in and out: positions, polls and the twin tables, zstd-compressed.

use std::sync::Arc;

use anyhow::{Context, anyhow};
use arrow::array::{
    Array, ArrayRef, AsArray, BooleanArray, Float32Array, Float64Array, Int32Array, Int64Array,
    RecordBatch, StringArray, UInt32Array,
};
use arrow::datatypes::{Float32Type, Float64Type, Int32Type, Int64Type};
use holdshort_core::{Obs, Poll, Src};
use holdshort_twin::classify::Kind;
use holdshort_twin::config::Bin;
use holdshort_twin::day::{Event, EventKind, Flight};
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
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

fn src_from(name: &str) -> anyhow::Result<Src> {
    match name {
        "adsblol" => Ok(Src::Adsblol),
        "opensky" => Ok(Src::Opensky),
        "history" => Ok(Src::History),
        other => Err(anyhow!("unknown src {other:?}")),
    }
}

/// Read positions written by `obs_parquet`.
pub fn read_obs(bytes: Vec<u8>) -> anyhow::Result<Vec<Obs>> {
    let reader = ParquetRecordBatchReaderBuilder::try_new(bytes::Bytes::from(bytes))?.build()?;
    let mut out = Vec::new();
    for batch in reader {
        let b = batch?;
        let col = |name: &str| {
            b.column_by_name(name)
                .with_context(|| format!("missing column {name}"))
        };
        // `as_*` views a column as its concrete Arrow type.
        let i64s = col("ts_ms")?.as_primitive::<Int64Type>().clone();
        let text = |name: &str| -> anyhow::Result<Vec<Option<String>>> {
            let a = col(name)?.as_string::<i32>().clone();
            Ok((0..a.len())
                .map(|i| (!a.is_null(i)).then(|| a.value(i).to_string()))
                .collect())
        };
        let f64s = |name: &str| -> anyhow::Result<Vec<f64>> {
            Ok(col(name)?.as_primitive::<Float64Type>().values().to_vec())
        };
        let opt_i32 = |name: &str| -> anyhow::Result<Vec<Option<i32>>> {
            let a = col(name)?.as_primitive::<Int32Type>().clone();
            Ok((0..a.len())
                .map(|i| (!a.is_null(i)).then(|| a.value(i)))
                .collect())
        };
        let opt_f32 = |name: &str| -> anyhow::Result<Vec<Option<f32>>> {
            let a = col(name)?.as_primitive::<Float32Type>().clone();
            Ok((0..a.len())
                .map(|i| (!a.is_null(i)).then(|| a.value(i)))
                .collect())
        };
        let (hex, flight, reg, ac_type) = (
            text("hex")?,
            text("flight")?,
            text("reg")?,
            text("ac_type")?,
        );
        let (squawk, category, src) = (text("squawk")?, text("category")?, text("src")?);
        let (lat, lon) = (f64s("lat")?, f64s("lon")?);
        let (baro, geom, vrate) = (
            opt_i32("alt_baro_ft")?,
            opt_i32("alt_geom_ft")?,
            opt_i32("vrate_fpm")?,
        );
        let (gs, track) = (opt_f32("gs_kt")?, opt_f32("track_deg")?);
        let ground = col("on_ground")?.as_boolean().clone();
        for i in 0..b.num_rows() {
            out.push(Obs {
                ts_ms: i64s.value(i),
                hex: hex[i].clone().unwrap_or_default(),
                flight: flight[i].clone(),
                reg: reg[i].clone(),
                ac_type: ac_type[i].clone(),
                lat: lat[i],
                lon: lon[i],
                alt_baro_ft: baro[i],
                alt_geom_ft: geom[i],
                on_ground: ground.value(i),
                gs_kt: gs[i],
                track_deg: track[i],
                vrate_fpm: vrate[i],
                squawk: squawk[i].clone(),
                category: category[i].clone(),
                src: src_from(src[i].as_deref().unwrap_or_default())?,
            });
        }
    }
    Ok(out)
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Arrival => "arrival",
        Kind::Departure => "departure",
        Kind::Both => "both",
        Kind::Overflight => "overflight",
        Kind::Other => "other",
    }
}

fn event_name(k: EventKind) -> &'static str {
    match k {
        EventKind::AsmaEntry => "asma_entry",
        EventKind::GoAround => "go_around",
        EventKind::HoldStart => "hold_start",
        EventKind::HoldEnd => "hold_end",
        EventKind::Touchdown => "touchdown",
        EventKind::Liftoff => "liftoff",
        EventKind::ConfigChange => "config_change",
    }
}

pub fn flights_parquet(fl: &[Flight]) -> anyhow::Result<Vec<u8>> {
    let s = |f: fn(&Flight) -> Option<&str>| -> ArrayRef {
        Arc::new(fl.iter().map(f).collect::<StringArray>())
    };
    let i = |f: fn(&Flight) -> Option<i64>| -> ArrayRef {
        Arc::new(fl.iter().map(f).collect::<Int64Array>())
    };
    let d = |f: fn(&Flight) -> Option<f64>| -> ArrayRef {
        Arc::new(fl.iter().map(f).collect::<Float64Array>())
    };
    write(vec![
        ("id", s(|f| Some(f.id.as_str()))),
        ("hex", s(|f| Some(f.hex.as_str()))),
        ("flight", s(|f| f.flight.as_deref())),
        ("reg", s(|f| f.reg.as_deref())),
        ("ac_type", s(|f| f.ac_type.as_deref())),
        ("first_ms", i(|f| Some(f.first_ms))),
        ("last_ms", i(|f| Some(f.last_ms))),
        ("kind", s(|f| Some(kind_name(f.kind)))),
        ("arr_runway", s(|f| f.arr_runway.as_deref())),
        ("touchdown_ms", i(|f| f.touchdown_ms)),
        ("dep_runway", s(|f| f.dep_runway.as_deref())),
        ("liftoff_ms", i(|f| f.liftoff_ms)),
        ("runway_margin_m", d(|f| f.runway_margin_m)),
        (
            "time_low_confidence",
            Arc::new(
                fl.iter()
                    .map(|f| Some(f.time_low_confidence))
                    .collect::<BooleanArray>(),
            ),
        ),
        (
            "go_arounds",
            Arc::new(fl.iter().map(|f| f.go_arounds).collect::<UInt32Array>()),
        ),
        (
            "holds",
            Arc::new(fl.iter().map(|f| f.holds).collect::<UInt32Array>()),
        ),
        ("holding_s", d(|f| Some(f.holding_s))),
        ("asma_entry_ms", i(|f| f.asma_entry_ms)),
        ("asma_s", d(|f| f.asma_s)),
    ])
}

pub fn events_parquet(ev: &[Event]) -> anyhow::Result<Vec<u8>> {
    let s = |f: fn(&Event) -> Option<&str>| -> ArrayRef {
        Arc::new(ev.iter().map(f).collect::<StringArray>())
    };
    let d = |f: fn(&Event) -> Option<f64>| -> ArrayRef {
        Arc::new(ev.iter().map(f).collect::<Float64Array>())
    };
    write(vec![
        ("flight_id", s(|e| e.flight_id.as_deref())),
        (
            "ms",
            Arc::new(ev.iter().map(|e| e.ms).collect::<Int64Array>()) as ArrayRef,
        ),
        ("kind", s(|e| Some(event_name(e.kind)))),
        ("runway", s(|e| e.runway.as_deref())),
        ("lat", d(|e| e.lat)),
        ("lon", d(|e| e.lon)),
        ("h_agl_ft", d(|e| e.h_agl_ft)),
        ("detail", s(|e| e.detail.as_deref())),
    ])
}

pub fn bins_parquet(bins: &[Bin]) -> anyhow::Result<Vec<u8>> {
    write(vec![
        (
            "bin_ms",
            Arc::new(bins.iter().map(|b| b.bin_ms).collect::<Int64Array>()) as ArrayRef,
        ),
        (
            "runway",
            Arc::new(
                bins.iter()
                    .map(|b| Some(b.runway.as_str()))
                    .collect::<StringArray>(),
            ),
        ),
        (
            "arrivals",
            Arc::new(bins.iter().map(|b| b.arrivals).collect::<UInt32Array>()),
        ),
        (
            "departures",
            Arc::new(bins.iter().map(|b| b.departures).collect::<UInt32Array>()),
        ),
        (
            "config",
            Arc::new(
                bins.iter()
                    .map(|b| Some(b.config.as_str()))
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
    fn read_obs_round_trip() {
        let v = vec![obs(Some("THY1")), obs(None)];
        assert_eq!(read_obs(obs_parquet(&v).unwrap()).unwrap(), v);
    }

    #[test]
    fn twin_tables_write() {
        use holdshort_twin::config::Bin;
        use holdshort_twin::day::{Event, EventKind, Flight};
        let f = Flight::empty("aaaaaa-1", 1);
        let e = Event {
            flight_id: None,
            ms: 1,
            kind: EventKind::ConfigChange,
            runway: None,
            lat: None,
            lon: None,
            h_agl_ft: None,
            detail: Some("ARR 35L / DEP 36".into()),
        };
        let b = Bin {
            bin_ms: 1,
            runway: "35L".into(),
            arrivals: 2,
            departures: 0,
            config: "x".into(),
        };
        for bytes in [
            flights_parquet(&[f]).unwrap(),
            events_parquet(&[e]).unwrap(),
            bins_parquet(&[b]).unwrap(),
        ] {
            assert!(bytes.starts_with(b"PAR1"));
        }
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
