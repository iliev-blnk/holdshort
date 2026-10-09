//! Rules for the live poll loop: which feed to poll, when, which 5-min
//! chunk file a poll belongs to, and how much of the last 24 h was captured.

use std::collections::BTreeSet;

use chrono::DateTime;
use serde::{Deserialize, Serialize};

use crate::Src;

pub const POLL_MS: i64 = 10_000;
pub const FALLBACK_POLL_MS: i64 = 30_000;
pub const RETRY_PRIMARY_MS: i64 = 600_000;
pub const METAR_MS: i64 = 1_800_000;
pub const SLOT_MS: i64 = 300_000;
pub const DAY_MS: i64 = 86_400_000;
/// Polls run this long after each grid point.
pub const POLL_OFFSET_MS: i64 = 250;
const WINDOW_MS: i64 = 10_000;

/// Failover and METAR timing state for the poll loop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IngestState {
    pub source: Src,
    pub primary_failures: u32,
    pub last_primary_try_ms: i64,
    pub last_metar_ms: i64,
}

impl Default for IngestState {
    fn default() -> Self {
        Self {
            source: Src::Adsblol,
            primary_failures: 0,
            last_primary_try_ms: 0,
            last_metar_ms: 0,
        }
    }
}

impl IngestState {
    /// Which feed to poll now. In fallback, adsb.lol gets a retry every 10 min.
    pub fn choose(&self, now_ms: i64) -> Src {
        if self.source == Src::Adsblol || now_ms - self.last_primary_try_ms >= RETRY_PRIMARY_MS {
            Src::Adsblol
        } else {
            Src::Opensky
        }
    }

    /// Record the outcome of a poll.
    // `&mut self`: this method may change the struct, and the borrow checker
    // guarantees nothing else reads it meanwhile.
    pub fn record(&mut self, tried: Src, ok: bool, now_ms: i64) {
        if tried != Src::Adsblol {
            return;
        }
        self.last_primary_try_ms = now_ms;
        if ok {
            self.primary_failures = 0;
            self.source = Src::Adsblol;
        } else {
            self.primary_failures += 1;
            if self.primary_failures >= 2 {
                self.source = Src::Opensky;
            }
        }
    }

    pub fn interval_ms(&self) -> i64 {
        match self.source {
            Src::Adsblol => POLL_MS,
            _ => FALLBACK_POLL_MS,
        }
    }

    pub fn metar_due(&self, now_ms: i64) -> bool {
        now_ms - self.last_metar_ms >= METAR_MS
    }
}

/// Start of the 5-min UTC slot containing `ts_ms`.
pub fn slot_of(ts_ms: i64) -> i64 {
    // `div_euclid` floors toward minus infinity, unlike `/`.
    ts_ms.div_euclid(SLOT_MS) * SLOT_MS
}

/// Key of a live chunk: `raw/live/YYYY-MM-DD/HHMM.ndjson`.
pub fn chunk_key(slot_ms: i64) -> String {
    let t = DateTime::from_timestamp_millis(slot_ms).expect("timestamp in range");
    t.format("raw/live/%Y-%m-%d/%H%M.ndjson").to_string()
}

/// Next poll time on a fixed grid (`interval` + `POLL_OFFSET_MS`), so the
/// poll's own run time never makes the loop drift.
pub fn next_poll_ms(now_ms: i64, interval_ms: i64) -> i64 {
    ((now_ms - POLL_OFFSET_MS).div_euclid(interval_ms) + 1) * interval_ms + POLL_OFFSET_MS
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Coverage {
    pub expected: u32,
    pub captured: u32,
    pub pct: f64,
}

/// Share of 10 s windows in the last 24 h `(now - 24h, now]` with an ok poll.
pub fn coverage(ok_poll_ts: &[i64], now_ms: i64) -> Coverage {
    let expected = (DAY_MS / WINDOW_MS) as u32;
    let windows: BTreeSet<i64> = ok_poll_ts
        .iter()
        .filter(|&&t| t > now_ms - DAY_MS && t <= now_ms)
        .map(|t| t.div_euclid(WINDOW_MS))
        .collect();
    let captured = windows.len() as u32;
    let pct = (captured as f64 / expected as f64 * 1000.0).round() / 10.0;
    Coverage {
        expected,
        captured,
        pct,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_791_561_600_000; // 2026-10-09 16:00:00 UTC

    #[test]
    fn one_failure_stays_primary() {
        let mut s = IngestState::default();
        s.record(Src::Adsblol, false, T0);
        assert_eq!(s.choose(T0 + 1), Src::Adsblol);
        assert_eq!(s.interval_ms(), POLL_MS);
    }

    #[test]
    fn two_failures_switch_to_opensky() {
        let mut s = IngestState::default();
        s.record(Src::Adsblol, false, T0);
        s.record(Src::Adsblol, false, T0 + POLL_MS);
        assert_eq!(s.source, Src::Opensky);
        assert_eq!(s.choose(T0 + POLL_MS + 1), Src::Opensky);
        assert_eq!(s.interval_ms(), FALLBACK_POLL_MS);
    }

    #[test]
    fn fallback_retries_primary_after_10_min() {
        let mut s = IngestState::default();
        s.record(Src::Adsblol, false, T0);
        s.record(Src::Adsblol, false, T0);
        s.record(Src::Opensky, true, T0 + 30_000);
        assert_eq!(s.choose(T0 + 599_999), Src::Opensky);
        assert_eq!(s.choose(T0 + 600_000), Src::Adsblol);
    }

    #[test]
    fn primary_success_switches_back() {
        let mut s = IngestState::default();
        s.record(Src::Adsblol, false, T0);
        s.record(Src::Adsblol, false, T0);
        s.record(Src::Adsblol, true, T0 + 600_000);
        assert_eq!(s.source, Src::Adsblol);
        assert_eq!(s.primary_failures, 0);
        assert_eq!(s.interval_ms(), POLL_MS);
    }

    #[test]
    fn failed_retry_stays_in_fallback() {
        let mut s = IngestState::default();
        s.record(Src::Adsblol, false, T0);
        s.record(Src::Adsblol, false, T0);
        s.record(Src::Adsblol, false, T0 + 600_000);
        assert_eq!(s.source, Src::Opensky);
        assert_eq!(
            s.choose(T0 + 600_001),
            Src::Opensky,
            "retry clock restarted"
        );
    }

    #[test]
    fn metar_due_every_30_min() {
        let s = IngestState {
            last_metar_ms: T0,
            ..IngestState::default()
        };
        assert!(!s.metar_due(T0 + METAR_MS - 1));
        assert!(s.metar_due(T0 + METAR_MS));
        assert!(IngestState::default().metar_due(T0));
    }

    #[test]
    fn chunk_key_format() {
        assert_eq!(chunk_key(T0), "raw/live/2026-10-09/1600.ndjson");
        assert_eq!(slot_of(1_791_561_899_999), T0);
        assert_eq!(slot_of(1_791_561_900_000), T0 + SLOT_MS);
        assert_eq!(
            chunk_key(1_791_417_600_000),
            "raw/live/2026-10-08/0000.ndjson"
        );
    }

    #[test]
    fn next_poll_on_grid() {
        assert_eq!(
            next_poll_ms(T0 + 3_456, POLL_MS),
            T0 + 10_000 + POLL_OFFSET_MS
        );
        // Fired slightly late at +10 260: next is the following grid point, no drift.
        assert_eq!(
            next_poll_ms(T0 + 10_260, POLL_MS),
            T0 + 20_000 + POLL_OFFSET_MS
        );
        // Not yet fired at +10 249: still aims at +10 250.
        assert_eq!(
            next_poll_ms(T0 + 10_249, POLL_MS),
            T0 + 10_000 + POLL_OFFSET_MS
        );
        assert_eq!(
            next_poll_ms(T0 + 1, FALLBACK_POLL_MS) % FALLBACK_POLL_MS,
            POLL_OFFSET_MS
        );
    }

    #[test]
    fn coverage_counts_windows() {
        let now = T0;
        let c = coverage(
            &[now - 1_000, now - 2_000, now - 15_000, now - DAY_MS],
            now,
        );
        assert_eq!(c.expected, 8640);
        assert_eq!(
            c.captured, 2,
            "two polls in one window count once; 24h-old is out"
        );
        assert_eq!(c.pct, 0.0);
    }

    #[test]
    fn coverage_full_day_is_100() {
        let now = T0;
        let polls: Vec<i64> = (0..8640).map(|i| now - i * 10_000).collect();
        let c = coverage(&polls, now);
        assert_eq!(c.captured, 8640);
        assert_eq!(c.pct, 100.0);
    }
}
