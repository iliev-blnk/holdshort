//! One day of backfill: skip if done, else stream → filter → gzip → upload.

use std::io::{Read, Seek, SeekFrom};

use chrono::NaiveDate;
use flate2::Compression;
use flate2::write::GzEncoder;

use crate::filter::{DayStats, filter_day};
use crate::store::Store;

#[derive(Debug)]
pub enum Outcome {
    Skipped,
    Done(DayStats),
}

pub fn history_key(day: NaiveDate) -> String {
    format!("raw/history/{day}.ndjson.gz")
}

/// Backfill one day. `open` is only called when the day must be (re)done.
/// The upload happens only after the whole day was filtered.
pub fn backfill_day<R: Read>(
    store: &Store,
    day: NaiveDate,
    force: bool,
    open: impl FnOnce() -> anyhow::Result<R>,
) -> anyhow::Result<Outcome> {
    let key = history_key(day);
    if !force && store.exists(&key)? {
        return Ok(Outcome::Skipped);
    }
    let tmp = tempfile::tempfile()?;
    let mut gz = GzEncoder::new(tmp, Compression::default());
    let stats = filter_day(open()?, &mut gz)?;
    let mut tmp = gz.finish()?;
    tmp.seek(SeekFrom::Start(0))?;
    let mut data = Vec::new();
    tmp.read_to_end(&mut data)?;
    store.put(&key, data)?;
    Ok(Outcome::Done(stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::tests::sample_tar;
    use std::io::Cursor;

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 8).unwrap()
    }

    #[test]
    fn uploads_new_day() {
        let store = Store::memory();
        let out = backfill_day(&store, day(), false, || Ok(Cursor::new(sample_tar()))).unwrap();
        let Outcome::Done(stats) = out else {
            panic!("expected Done")
        };
        assert_eq!(stats.kept_obs, 3);
        assert!(store.exists("raw/history/2026-10-08.ndjson.gz").unwrap());
    }

    #[test]
    fn skip_existing_day() {
        let store = Store::memory();
        store
            .put("raw/history/2026-10-08.ndjson.gz", b"x".to_vec())
            .unwrap();
        let out = backfill_day(
            &store,
            day(),
            false,
            || -> anyhow::Result<Cursor<Vec<u8>>> { panic!("must not download") },
        )
        .unwrap();
        assert!(matches!(out, Outcome::Skipped));
    }

    #[test]
    fn force_redoes_day() {
        let store = Store::memory();
        store
            .put("raw/history/2026-10-08.ndjson.gz", b"x".to_vec())
            .unwrap();
        let out = backfill_day(&store, day(), true, || Ok(Cursor::new(sample_tar()))).unwrap();
        assert!(matches!(out, Outcome::Done(_)));
        assert_ne!(store.get("raw/history/2026-10-08.ndjson.gz").unwrap(), b"x");
    }

    #[test]
    fn failed_download_uploads_nothing() {
        let store = Store::memory();
        let r = backfill_day(
            &store,
            day(),
            false,
            || -> anyhow::Result<Cursor<Vec<u8>>> { anyhow::bail!("network down") },
        );
        assert!(r.is_err());
        assert!(!store.exists("raw/history/2026-10-08.ndjson.gz").unwrap());
    }
}
