//! Walk a history tar and keep only trace points inside the LTFM area.

use std::io::{Read, Write};

use flate2::read::GzDecoder;
use holdshort_core::airport::Airport;
use holdshort_core::history::parse_trace;
use holdshort_core::{Line, to_ndjson};

#[derive(Debug, Default, Clone, PartialEq)]
pub struct DayStats {
    pub traces: u64,
    pub bad_traces: u64,
    /// Points kept per airport, in the order the outputs were given.
    pub kept: Vec<u64>,
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

/// Stream a day's tar once; write each airport's in-area points as NDJSON
/// `obs` lines to that airport's output.
// Generics: the same code takes a file, a network stream or a test buffer.
pub fn filter_day<R: Read, W: Write>(
    tar: R,
    outs: &mut [(&Airport, W)],
) -> anyhow::Result<DayStats> {
    let mut stats = DayStats {
        kept: vec![0; outs.len()],
        ..DayStats::default()
    };
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
        let Ok(obs) = parsed else {
            stats.bad_traces += 1;
            continue;
        };
        for (i, (airport, out)) in outs.iter_mut().enumerate() {
            let lines: Vec<Line> = obs
                .iter()
                .filter(|o| airport.contains(o.lat, o.lon))
                .cloned()
                .map(Line::Obs)
                .collect();
            if !lines.is_empty() {
                stats.kept[i] += lines.len() as u64;
                out.write_all(to_ndjson(&lines).as_bytes())?;
            }
        }
    }
    Ok(stats)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use holdshort_core::airport::{LRSV, LTFM};
    use holdshort_core::{Line, from_ndjson};
    use std::io::{Cursor, Write};

    const NEAR: &str = include_str!("../../../fixtures/trace_small.json");
    const FAR: &str = include_str!("../../../fixtures/trace_full_sample.json");
    const SUCEAVA: &str = include_str!("../../../fixtures/trace_lrsv.json");

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
        add("./traces/01/trace_full_4a1001.json", &gz(SUCEAVA));
        add(
            "./traces/11/trace_full_badbad.json",
            &[0x1f, 0x8b, 0, 1, 2, 3],
        );
        b.into_inner().unwrap()
    }

    fn obs_hexes(out: &[u8]) -> Vec<String> {
        from_ndjson(std::str::from_utf8(out).unwrap())
            .unwrap()
            .into_iter()
            .map(|l| match l {
                Line::Obs(o) => o.hex,
                _ => panic!("only obs lines"),
            })
            .collect()
    }

    #[test]
    fn filter_day_keeps_only_ist() {
        let mut outs = [(&LTFM, Vec::new())];
        let stats = filter_day(Cursor::new(sample_tar()), &mut outs).unwrap();
        assert_eq!(stats.traces, 4);
        assert_eq!(stats.bad_traces, 1);
        assert_eq!(stats.kept, vec![3]);
        assert_eq!(obs_hexes(&outs[0].1), vec!["4bb208"; 3]);
    }

    #[test]
    fn one_pass_feeds_both_airports() {
        let mut outs = [(&LTFM, Vec::new()), (&LRSV, Vec::new())];
        let stats = filter_day(Cursor::new(sample_tar()), &mut outs).unwrap();
        assert_eq!(stats.kept, vec![3, 2]);
        assert_eq!(obs_hexes(&outs[0].1), vec!["4bb208"; 3]);
        assert_eq!(obs_hexes(&outs[1].1), vec!["4a1001"; 2]);
    }

    #[test]
    fn filter_day_reads_split_parts() {
        let tar = sample_tar();
        let (a, b) = tar.split_at(1234);
        let mut whole = [(&LTFM, Vec::new()), (&LRSV, Vec::new())];
        filter_day(Cursor::new(tar.clone()), &mut whole).unwrap();
        let mut split = [(&LTFM, Vec::new()), (&LRSV, Vec::new())];
        filter_day(
            Cursor::new(a.to_vec()).chain(Cursor::new(b.to_vec())),
            &mut split,
        )
        .unwrap();
        assert_eq!(whole[0].1, split[0].1);
        assert_eq!(whole[1].1, split[1].1);
    }
}
