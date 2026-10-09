//! One day of backfill: skip airports already done, else stream the day once
//! → filter per airport → gzip → upload.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use chrono::NaiveDate;
use flate2::Compression;
use flate2::write::GzEncoder;
use holdshort_core::airport::Airport;

use crate::filter::{DayStats, filter_day};
use crate::store::Store;

#[derive(Debug)]
pub enum Outcome {
    Skipped,
    Done {
        stats: DayStats,
        airports: Vec<&'static str>,
    },
}

pub fn history_key(airport: &Airport, day: NaiveDate) -> String {
    format!("{}/raw/history/{day}.ndjson.gz", airport.icao)
}

/// Backfill one day for every airport that lacks it (all of them with
/// `force`). `open` is called at most once. Files are written only after the
/// whole day was filtered.
pub fn backfill_day<R: Read>(
    store: &Store,
    airports: &[&'static Airport],
    day: NaiveDate,
    force: bool,
    open: impl FnOnce() -> anyhow::Result<R>,
) -> anyhow::Result<Outcome> {
    let mut todo = Vec::new();
    for &a in airports {
        if force || !store.exists(&history_key(a, day))? {
            todo.push(a);
        }
    }
    if todo.is_empty() {
        return Ok(Outcome::Skipped);
    }
    let mut outs: Vec<(&Airport, GzEncoder<File>)> = Vec::new();
    for &a in &todo {
        outs.push((
            a,
            GzEncoder::new(tempfile::tempfile()?, Compression::default()),
        ));
    }
    let stats = filter_day(open()?, &mut outs)?;
    for (a, gz) in outs {
        let mut tmp = gz.finish()?;
        tmp.seek(SeekFrom::Start(0))?;
        let mut data = Vec::new();
        tmp.read_to_end(&mut data)?;
        store.put(&history_key(a, day), data)?;
    }
    Ok(Outcome::Done {
        stats,
        airports: todo.iter().map(|a| a.icao).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::tests::sample_tar;
    use holdshort_core::airport::{LRSV, LTFM};
    use std::io::Cursor;

    const BOTH: &[&Airport] = &[&LTFM, &LRSV];

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 8).unwrap()
    }

    fn no_download() -> anyhow::Result<Cursor<Vec<u8>>> {
        panic!("must not download")
    }

    #[test]
    fn uploads_new_day_for_each_airport() {
        let store = Store::memory();
        let out =
            backfill_day(&store, BOTH, day(), false, || Ok(Cursor::new(sample_tar()))).unwrap();
        let Outcome::Done { stats, airports } = out else {
            panic!("expected Done")
        };
        assert_eq!(stats.kept, vec![3, 2]);
        assert_eq!(airports, vec!["LTFM", "LRSV"]);
        assert!(
            store
                .exists("LTFM/raw/history/2026-10-08.ndjson.gz")
                .unwrap()
        );
        assert!(
            store
                .exists("LRSV/raw/history/2026-10-08.ndjson.gz")
                .unwrap()
        );
    }

    #[test]
    fn skip_when_every_airport_has_the_day() {
        let store = Store::memory();
        for a in BOTH {
            store.put(&history_key(a, day()), b"x".to_vec()).unwrap();
        }
        let out = backfill_day(&store, BOTH, day(), false, no_download).unwrap();
        assert!(matches!(out, Outcome::Skipped));
    }

    #[test]
    fn only_missing_airports_are_done() {
        let store = Store::memory();
        store
            .put(&history_key(&LTFM, day()), b"x".to_vec())
            .unwrap();
        let out =
            backfill_day(&store, BOTH, day(), false, || Ok(Cursor::new(sample_tar()))).unwrap();
        let Outcome::Done { airports, .. } = out else {
            panic!("expected Done")
        };
        assert_eq!(airports, vec!["LRSV"]);
        assert_eq!(store.get(&history_key(&LTFM, day())).unwrap(), b"x");
    }

    #[test]
    fn force_redoes_day() {
        let store = Store::memory();
        store
            .put(&history_key(&LTFM, day()), b"x".to_vec())
            .unwrap();
        let out = backfill_day(&store, &[&LTFM], day(), true, || {
            Ok(Cursor::new(sample_tar()))
        })
        .unwrap();
        assert!(matches!(out, Outcome::Done { .. }));
        assert_ne!(store.get(&history_key(&LTFM, day())).unwrap(), b"x");
    }

    #[test]
    fn failed_download_uploads_nothing() {
        let store = Store::memory();
        let r = backfill_day(
            &store,
            BOTH,
            day(),
            false,
            || -> anyhow::Result<Cursor<Vec<u8>>> { anyhow::bail!("network down") },
        );
        assert!(r.is_err());
        assert!(!store.exists(&history_key(&LTFM, day())).unwrap());
    }
}
