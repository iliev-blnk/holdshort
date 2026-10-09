//! Monthly METAR files `<ICAO>/weather/YYYY-MM.ndjson` and the IEM history fetch.

use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, NaiveDate};
use holdshort_core::airport::Airport;
use holdshort_core::metar::parse_iem_csv;
use holdshort_core::{Line, Metar, from_ndjson, to_ndjson};

use crate::UA;
use crate::store::Store;

/// Key of the airport's METAR file for the month containing `ts_ms`.
pub fn month_key(airport: &Airport, ts_ms: i64) -> String {
    let t = DateTime::from_timestamp_millis(ts_ms).expect("timestamp in range");
    format!("{}/weather/{}", airport.icao, t.format("%Y-%m.ndjson"))
}

/// Union of an existing monthly file and new reports; existing rows win,
/// output sorted by observation time.
pub fn merge_month(existing: &str, new: Vec<Metar>) -> anyhow::Result<String> {
    let mut by_ts: BTreeMap<i64, Metar> = BTreeMap::new();
    let old = from_ndjson(existing)?.into_iter().filter_map(|l| match l {
        Line::Metar(m) => Some(m),
        _ => None,
    });
    for m in old.chain(new) {
        by_ts.entry(m.obs_ts_ms).or_insert(m);
    }
    let lines: Vec<Line> = by_ts.into_values().map(Line::Metar).collect();
    Ok(to_ndjson(&lines))
}

/// Merge reports into their monthly files in the store.
pub fn store_metars(store: &Store, airport: &Airport, metars: Vec<Metar>) -> anyhow::Result<usize> {
    let mut by_month: BTreeMap<String, Vec<Metar>> = BTreeMap::new();
    for m in metars {
        by_month
            .entry(month_key(airport, m.obs_ts_ms))
            .or_default()
            .push(m);
    }
    let n = by_month.len();
    for (key, ms) in by_month {
        let existing = if store.exists(&key)? {
            String::from_utf8(store.get(&key)?)?
        } else {
            String::new()
        };
        store.put(&key, merge_month(&existing, ms)?.into_bytes())?;
    }
    Ok(n)
}

/// Iowa Mesonet METAR archive for the airport, `from..=to` (UTC days).
pub fn fetch_iem(airport: &Airport, from: NaiveDate, to: NaiveDate) -> anyhow::Result<Vec<Metar>> {
    // IEM's end date is exclusive.
    let end = to.succ_opt().expect("date in range");
    let url = format!(
        "https://mesonet.agron.iastate.edu/cgi-bin/request/asos.py?station={}&data=metar\
         &year1={}&month1={}&day1={}&year2={}&month2={}&day2={}&tz=Etc/UTC&format=onlycomma\
         &latlon=no&missing=M&trace=T&direct=no&report_type=3&report_type=4",
        airport.icao,
        from.year(),
        from.month(),
        from.day(),
        end.year(),
        end.month(),
        end.day()
    );
    let body = reqwest::blocking::Client::builder()
        .user_agent(UA)
        .build()?
        .get(url)
        .send()?
        .error_for_status()?
        .text()?;
    Ok(parse_iem_csv(&body)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(ts: i64, raw: &str) -> Metar {
        Metar {
            station: "LTFM".into(),
            obs_ts_ms: ts,
            raw: raw.into(),
        }
    }

    #[test]
    fn merge_month_dedupes_and_sorts() {
        let existing = holdshort_core::to_ndjson(&[Line::Metar(m(3, "c")), Line::Metar(m(1, "a"))]);
        let merged = merge_month(&existing, vec![m(2, "b"), m(1, "a-dup")]).unwrap();
        let back: Vec<i64> = holdshort_core::from_ndjson(&merged)
            .unwrap()
            .into_iter()
            .map(|l| match l {
                Line::Metar(x) => x.obs_ts_ms,
                _ => panic!("metar only"),
            })
            .collect();
        assert_eq!(back, vec![1, 2, 3]);
        assert!(merged.contains("\"raw\":\"a\""), "existing row wins");
    }

    #[test]
    fn month_key_format() {
        assert_eq!(
            month_key(&holdshort_core::airport::LRSV, 1_791_417_600_000),
            "LRSV/weather/2026-10.ndjson"
        );
    }
}
