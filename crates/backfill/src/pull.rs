//! `holdshort pull`: copy a day's live chunks from D1 into the local store.

use chrono::NaiveDate;

use crate::d1::hour_range;
use crate::store::Store;

/// Fetch each UTC hour of `day` (plus hour 00 of the next day, for midnight
/// stragglers) with `query(from_key, to_key)` and store chunks not yet
/// present locally. Returns how many were written.
pub fn pull_day(
    store: &Store,
    day: NaiveDate,
    mut query: impl FnMut(&str, &str) -> anyhow::Result<Vec<(String, String)>>,
) -> anyhow::Result<usize> {
    let next = day.succ_opt().expect("date in range");
    let hours = (0..24).map(|h| (day, h)).chain(std::iter::once((next, 0)));
    let mut written = 0;
    for (d, h) in hours {
        let (from, to) = hour_range(&d.to_string(), h);
        for (key, body) in query(&from, &to)? {
            if !store.exists(&key)? {
                store.put(&key, body.into_bytes())?;
                written += 1;
            }
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 8).unwrap()
    }

    #[test]
    fn pulls_new_chunks_and_skips_existing() {
        let store = Store::memory();
        store
            .put("raw/live/2026-10-08/1600.ndjson", b"old".to_vec())
            .unwrap();
        let mut asked = Vec::new();
        let n = pull_day(&store, day(), |from, to| {
            asked.push((from.to_string(), to.to_string()));
            if from.ends_with("2026-10-08/16") {
                Ok(vec![
                    ("raw/live/2026-10-08/1600.ndjson".into(), "new".into()),
                    ("raw/live/2026-10-08/1605.ndjson".into(), "b".into()),
                ])
            } else {
                Ok(vec![])
            }
        })
        .unwrap();
        assert_eq!(n, 1);
        assert_eq!(
            store.get("raw/live/2026-10-08/1600.ndjson").unwrap(),
            b"old"
        );
        assert_eq!(store.get("raw/live/2026-10-08/1605.ndjson").unwrap(), b"b");
        // 24 hours of the day, plus hour 00 of the next day for stragglers.
        assert_eq!(asked.len(), 25);
        assert_eq!(asked[24].0, "raw/live/2026-10-09/00");
    }

    #[test]
    fn query_error_fails_the_day() {
        let store = Store::memory();
        assert!(pull_day(&store, day(), |_, _| anyhow::bail!("http 500")).is_err());
    }
}
