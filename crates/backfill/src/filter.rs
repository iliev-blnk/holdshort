//! Walk a history tar and keep only trace points inside the LTFM area.

use std::io::{Read, Write};

use flate2::read::GzDecoder;
use holdshort_core::area::retain_in_area;
use holdshort_core::history::parse_trace;
use holdshort_core::{Line, to_ndjson};

#[derive(Debug, Default, Clone, PartialEq)]
pub struct DayStats {
    pub traces: u64,
    pub kept_obs: u64,
    pub bad_traces: u64,
}

fn is_trace(path: &str) -> bool {
    path.contains("traces/")
        && path
            .rsplit('/')
            .next()
            .is_some_and(|f| f.starts_with("trace_full_") && f.ends_with(".json"))
}

fn decode(raw: &[u8]) -> std::io::Result<String> {
    let mut s = String::new();
    if raw.starts_with(&[0x1f, 0x8b]) {
        GzDecoder::new(raw).read_to_string(&mut s)?;
    } else {
        s = String::from_utf8_lossy(raw).into_owned();
    }
    Ok(s)
}

/// Stream a day's tar; write in-area points as NDJSON `obs` lines to `out`.
// `impl Read` / generics: the same code takes a file, a network stream or a test buffer.
pub fn filter_day<R: Read, W: Write>(tar: R, mut out: W) -> anyhow::Result<DayStats> {
    let mut stats = DayStats::default();
    let mut archive = tar::Archive::new(tar);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().into_owned();
        if !is_trace(&path) {
            continue; // the tar reader skips the body for us
        }
        stats.traces += 1;
        let mut raw = Vec::new();
        entry.read_to_end(&mut raw)?;
        let parsed = decode(&raw)
            .map_err(anyhow::Error::from)
            .and_then(|json| parse_trace(&json).map_err(anyhow::Error::from));
        let mut obs = match parsed {
            Ok(obs) => obs,
            Err(_) => {
                stats.bad_traces += 1;
                continue;
            }
        };
        retain_in_area(&mut obs);
        if obs.is_empty() {
            continue;
        }
        stats.kept_obs += obs.len() as u64;
        let lines: Vec<Line> = obs.into_iter().map(Line::Obs).collect();
        out.write_all(to_ndjson(&lines).as_bytes())?;
    }
    Ok(stats)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use holdshort_core::area::in_area;
    use holdshort_core::{Line, from_ndjson};
    use std::io::{Cursor, Write};

    const NEAR: &str = include_str!("../../../fixtures/trace_small.json");
    const FAR: &str = include_str!("../../../fixtures/trace_full_sample.json");

    fn gz(s: &str) -> Vec<u8> {
        let mut e = GzEncoder::new(Vec::new(), Compression::default());
        e.write_all(s.as_bytes()).unwrap();
        e.finish().unwrap()
    }

    /// A tar shaped like a real day: heatmap first, then gzip traces.
    pub(crate) fn sample_tar() -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        let mut add = |path: &str, data: &[u8]| {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, path, data).unwrap();
        };
        add("./heatmap/00.bin.ttf", &[7u8; 3000]);
        add("./traces/08/trace_full_4bb208.json", &gz(NEAR));
        add("./traces/49/trace_full_~2b9049.json", &gz(FAR));
        add(
            "./traces/11/trace_full_badbad.json",
            &[0x1f, 0x8b, 0, 1, 2, 3],
        );
        b.into_inner().unwrap()
    }

    #[test]
    fn filter_day_keeps_only_ist() {
        let mut out = Vec::new();
        let stats = filter_day(Cursor::new(sample_tar()), &mut out).unwrap();
        let lines = from_ndjson(std::str::from_utf8(&out).unwrap()).unwrap();
        assert_eq!(stats.traces, 3);
        assert_eq!(stats.bad_traces, 1);
        assert_eq!(stats.kept_obs, 3);
        assert_eq!(lines.len(), 3);
        for l in lines {
            let Line::Obs(o) = l else {
                panic!("only obs lines")
            };
            assert!(in_area(o.lat, o.lon));
            assert_eq!(o.hex, "4bb208");
        }
    }

    #[test]
    fn filter_day_reads_split_parts() {
        let tar = sample_tar();
        let (a, b) = tar.split_at(1234);
        let mut whole = Vec::new();
        filter_day(Cursor::new(tar.clone()), &mut whole).unwrap();
        let mut split = Vec::new();
        filter_day(
            Cursor::new(a.to_vec()).chain(Cursor::new(b.to_vec())),
            &mut split,
        )
        .unwrap();
        assert_eq!(whole, split);
    }
}
