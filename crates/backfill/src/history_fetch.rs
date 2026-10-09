//! Locate and stream one day of adsb.lol history from GitHub releases.

use std::io::Read;

use anyhow::anyhow;
use chrono::{Datelike, NaiveDate};
use serde::Deserialize;

use crate::UA;

#[derive(Deserialize)]
struct Release {
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// Release tag of the production pod for a day.
pub fn tag(day: NaiveDate) -> String {
    day.format("v%Y.%m.%d-planes-readsb-prod-0").to_string()
}

/// Download URLs of the day's tar parts (`.tar.aa`, `.tar.ab`, ...) in order.
pub fn part_urls(release_json: &str, day: NaiveDate) -> anyhow::Result<Vec<String>> {
    let rel: Release = serde_json::from_str(release_json)?;
    let prefix = format!("{}.tar.a", tag(day));
    let mut parts: Vec<Asset> = rel
        .assets
        .into_iter()
        .filter(|a| a.name.starts_with(&prefix) && a.name.len() == prefix.len() + 1)
        .collect();
    if parts.is_empty() {
        return Err(anyhow!("no history release for {day}"));
    }
    parts.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(parts.into_iter().map(|a| a.browser_download_url).collect())
}

/// Reads one remote object, reopening at the current byte offset when the
/// connection breaks. `open(pos)` must return the object from byte `pos`.
/// Gives up after `max_retries` consecutive failures without progress.
pub struct ResumingReader<F: FnMut(u64) -> std::io::Result<Box<dyn Read>>> {
    open: F,
    inner: Option<Box<dyn Read>>,
    pos: u64,
    max_retries: u32,
    failures: u32,
    backoff: std::time::Duration,
}

impl<F: FnMut(u64) -> std::io::Result<Box<dyn Read>>> ResumingReader<F> {
    /// Waits `backoff × failures` before each reopen.
    pub fn new(open: F, max_retries: u32, backoff: std::time::Duration) -> Self {
        Self {
            open,
            inner: None,
            pos: 0,
            max_retries,
            failures: 0,
            backoff,
        }
    }
}

impl<F: FnMut(u64) -> std::io::Result<Box<dyn Read>>> Read for ResumingReader<F> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let attempt = match self.inner.as_mut() {
                Some(r) => r.read(buf),
                None => (self.open)(self.pos)
                    .map(|r| {
                        self.inner = Some(r);
                        0
                    })
                    .and_then(|_| self.inner.as_mut().expect("just opened").read(buf)),
            };
            match attempt {
                Ok(n) => {
                    if n > 0 {
                        self.failures = 0;
                    }
                    self.pos += n as u64;
                    return Ok(n);
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    self.inner = None;
                    self.failures += 1;
                    if self.failures > self.max_retries {
                        return Err(e);
                    }
                    eprintln!("  connection broke at byte {} ({e}); resuming", self.pos);
                    std::thread::sleep(self.backoff * self.failures);
                }
            }
        }
    }
}

/// GET `url` from byte `pos` (a Range request when `pos > 0`).
fn open_at(
    client: &reqwest::blocking::Client,
    url: &str,
    pos: u64,
) -> std::io::Result<Box<dyn Read>> {
    let mut req = client.get(url);
    if pos > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={pos}-"));
    }
    let resp = req.send().map_err(std::io::Error::other)?;
    let status = resp.status();
    if pos > 0 && status != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(std::io::Error::other(format!(
            "resume at {pos}: http {status}"
        )));
    }
    if !status.is_success() {
        return Err(std::io::Error::other(format!(
            "download {url}: http {status}"
        )));
    }
    Ok(Box::new(resp))
}

/// One continuous reader over all parts of a day, never touching disk.
pub fn open_day(day: NaiveDate) -> anyhow::Result<Box<dyn Read>> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(UA)
        .timeout(None)
        .build()?;
    let api = format!(
        "https://api.github.com/repos/adsblol/globe_history_{}/releases/tags/{}",
        day.year(),
        tag(day)
    );
    let mut req = client.get(&api);
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        req = req.bearer_auth(token);
    }
    let resp = req.send()?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(anyhow!("no history release for {day}"));
    }
    let json = resp.error_for_status()?.text()?;
    // `Box<dyn Read>` erases the concrete type so parts can be chained in a loop.
    let mut reader: Box<dyn Read> = Box::new(std::io::empty());
    for url in part_urls(&json, day)? {
        let client = client.clone();
        let part = ResumingReader::new(
            move |pos| open_at(&client, &url, pos),
            5,
            std::time::Duration::from_secs(2),
        );
        reader = Box::new(reader.chain(part));
    }
    Ok(reader)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 8).unwrap()
    }

    #[test]
    fn parts_sorted_by_suffix() {
        let json = r#"{"tag_name":"v2026.10.08-planes-readsb-prod-0","assets":[
            {"name":"v2026.10.08-planes-readsb-prod-0.tar.ac","browser_download_url":"u/ac"},
            {"name":"v2026.10.08-planes-readsb-staging-0.tar.aa","browser_download_url":"u/staging"},
            {"name":"v2026.10.08-planes-readsb-prod-0.tar.aa","browser_download_url":"u/aa"},
            {"name":"v2026.10.08-planes-readsb-prod-0.tar.ab","browser_download_url":"u/ab"}]}"#;
        assert_eq!(
            part_urls(json, day()).unwrap(),
            vec!["u/aa", "u/ab", "u/ac"]
        );
    }

    #[test]
    fn missing_release_is_error() {
        let json = r#"{"tag_name":"x","assets":[]}"#;
        let err = part_urls(json, day()).unwrap_err().to_string();
        assert!(err.contains("no history release for 2026-10-08"), "{err}");
    }

    /// A source that serves `data[pos..]` but fails after `chunk` bytes for
    /// the first `failures` opens.
    fn flaky(
        data: Vec<u8>,
        failures: u32,
        chunk: usize,
    ) -> impl FnMut(u64) -> std::io::Result<Box<dyn Read>> {
        let mut opens = 0;
        move |pos| {
            opens += 1;
            let rest = data[pos as usize..].to_vec();
            if opens <= failures {
                let cut = rest[..chunk.min(rest.len())].to_vec();
                Ok(Box::new(std::io::Cursor::new(cut).chain(Broken)) as Box<dyn Read>)
            } else {
                Ok(Box::new(std::io::Cursor::new(rest)) as Box<dyn Read>)
            }
        }
    }

    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("stream reset"))
        }
    }

    #[test]
    fn resuming_reader_recovers_mid_stream() {
        let data: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        let mut r = ResumingReader::new(flaky(data.clone(), 3, 1000), 5, Duration::ZERO);
        let mut got = Vec::new();
        r.read_to_end(&mut got).unwrap();
        assert_eq!(got, data);
    }

    #[test]
    fn resuming_reader_gives_up_without_progress() {
        let mut r = ResumingReader::new(flaky(vec![1u8; 100], 100, 0), 3, Duration::ZERO);
        assert!(r.read_to_end(&mut Vec::new()).is_err());
    }

    #[test]
    fn progress_resets_retry_budget() {
        // 100 failures in total, but each attempt makes progress first.
        let mut r = ResumingReader::new(flaky(vec![1u8; 10_000], 100, 10), 3, Duration::ZERO);
        let mut got = Vec::new();
        r.read_to_end(&mut got).unwrap();
        assert_eq!(got.len(), 10_000);
    }

    #[test]
    fn tag_format() {
        assert_eq!(tag(day()), "v2026.10.08-planes-readsb-prod-0");
    }
}
