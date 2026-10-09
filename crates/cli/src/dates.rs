//! `2026-09-01` or `2026-09-01..2026-09-30` (inclusive).

use anyhow::{Context, bail};
pub use chrono::NaiveDate;

fn date(s: &str) -> anyhow::Result<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").with_context(|| format!("bad date {s:?}"))
}

/// All days in a single date or an inclusive `from..to` range.
pub fn parse_range(s: &str) -> anyhow::Result<Vec<NaiveDate>> {
    let (from, to) = match s.split_once("..") {
        Some((a, b)) => (date(a)?, date(b)?),
        None => (date(s)?, date(s)?),
    };
    if to < from {
        bail!("range end {to} is before start {from}");
    }
    Ok(from.iter_days().take_while(|d| *d <= to).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn range_single() {
        assert_eq!(parse_range("2026-10-08").unwrap(), vec![d("2026-10-08")]);
    }

    #[test]
    fn range_inclusive() {
        let r = parse_range("2026-09-30..2026-10-02").unwrap();
        assert_eq!(r, vec![d("2026-09-30"), d("2026-10-01"), d("2026-10-02")]);
    }

    #[test]
    fn range_reversed_is_error() {
        assert!(parse_range("2026-10-02..2026-10-01").is_err());
    }

    #[test]
    fn range_garbage_is_error() {
        assert!(parse_range("yesterday").is_err());
    }
}
