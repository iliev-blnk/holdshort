//! Rules for the live ingest loop: which feed to poll, when, which 5-min
//! chunks to flush, and how much of the last 24 h was captured.

use std::collections::BTreeSet;

use chrono::DateTime;
use serde::{Deserialize, Serialize};

use crate::Src;

pub const POLL_MS: i64 = 10_000;
pub const FALLBACK_POLL_MS: i64 = 30_000;
pub const RETRY_PRIMARY_MS: i64 = 600_000;
pub const METAR_MS: i64 = 1_800_000;
pub const SLOT_MS: i64 = 300_000;
pub const BUFFER_MS: i64 = 86_400_000;
/// Slots flushed per alarm: one keeps every alarm small; catch-up still runs
/// at 30x real time.
pub const MAX_FLUSH: usize = 1;
/// Alarms fire this long after each grid point.
pub const ALARM_OFFSET_MS: i64 = 250;
/// How often the cached `/health` coverage is recomputed.
pub const HEALTH_MS: i64 = 900_000;
const WINDOW_MS: i64 = 10_000;

/// Persisted between alarms (as JSON in the Durable Object).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IngestState {
    pub source: Src,
    pub primary_failures: u32,
    pub last_primary_try_ms: i64,
    pub last_metar_ms: i64,
    #[serde(default)]
    pub last_health_ms: i64,
}

impl Default for IngestState {
    fn default() -> Self {
        Self {
            source: Src::Adsblol,
            primary_failures: 0,
            last_primary_try_ms: 0,
            last_metar_ms: 0,
            last_health_ms: 0,
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

    pub fn health_due(&self, now_ms: i64) -> bool {
        now_ms - self.last_health_ms >= HEALTH_MS
    }

    /// Whether this state differs from `before` in a way worth a storage
    /// write. A routine adsb.lol poll only moves `last_primary_try_ms`, which
    /// matters only in fallback, so it is not saved then.
    pub fn needs_save(&self, before: &IngestState) -> bool {
        let routine = self.source == Src::Adsblol && before.source == Src::Adsblol;
        if routine {
            let mut a = self.clone();
            a.last_primary_try_ms = before.last_primary_try_ms;
            a != *before
        } else {
            self != before
        }
    }
}

/// Start of the 5-min UTC slot containing `ts_ms`.
pub fn slot_of(ts_ms: i64) -> i64 {
    // `div_euclid` floors toward minus infinity, unlike `/`.
    ts_ms.div_euclid(SLOT_MS) * SLOT_MS
}

/// R2 key of a live chunk: `raw/live/YYYY-MM-DD/HHMM.ndjson`.
pub fn chunk_key(slot_ms: i64) -> String {
    let t = DateTime::from_timestamp_millis(slot_ms).expect("timestamp in range");
    t.format("raw/live/%Y-%m-%d/%H%M.ndjson").to_string()
}

/// D1 rejects rows over 2 MB; chunk parts stay under this.
pub const MAX_PART_BYTES: usize = 1_500_000;

/// Key of part `n` of a slot's chunk: part 0 is `chunk_key`, later parts
/// get a `-n` suffix (`raw/live/YYYY-MM-DD/HHMM-n.ndjson`).
pub fn chunk_part_key(slot_ms: i64, part: usize) -> String {
    let key = chunk_key(slot_ms);
    if part == 0 {
        key
    } else {
        key.replace(".ndjson", &format!("-{part}.ndjson"))
    }
}

/// Split NDJSON into parts of at most `max_bytes`, on line boundaries. A
/// single longer line becomes its own part.
pub fn split_chunk(body: &str, max_bytes: usize) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    // `split_inclusive` keeps each line's trailing newline.
    for line in body.split_inclusive('\n') {
        if !cur.is_empty() && cur.len() + line.len() > max_bytes {
            parts.push(std::mem::take(&mut cur));
        }
        cur.push_str(line);
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}

/// What to do with the oldest buffered slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferAction {
    /// Older than the 24 h cap: delete it and record a gap.
    Drop(i64),
    /// Finished: write it to R2.
    Flush(i64),
    Done,
}

/// Decide the next buffer step from the oldest buffered slot and how many
/// slots this alarm already flushed. Dropping ignores the flush cap.
pub fn next_buffer_action(oldest: Option<i64>, now_ms: i64, flushed: usize) -> BufferAction {
    match oldest {
        Some(slot) if slot < now_ms - BUFFER_MS => BufferAction::Drop(slot),
        Some(slot) if slot < slot_of(now_ms) && flushed < MAX_FLUSH => BufferAction::Flush(slot),
        _ => BufferAction::Done,
    }
}

/// Next alarm time on a fixed grid (`interval` + `ALARM_OFFSET_MS`), so the
/// handler's own run time never makes the loop drift.
pub fn next_alarm_ms(now_ms: i64, interval_ms: i64) -> i64 {
    ((now_ms - ALARM_OFFSET_MS).div_euclid(interval_ms) + 1) * interval_ms + ALARM_OFFSET_MS
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Coverage {
    pub expected: u32,
    pub captured: u32,
    pub pct: f64,
}

/// Share of 10 s windows in the last 24 h `(now - 24h, now]` with an ok poll.
pub fn coverage(ok_poll_ts: &[i64], now_ms: i64) -> Coverage {
    let expected = (BUFFER_MS / WINDOW_MS) as u32;
    let windows: BTreeSet<i64> = ok_poll_ts
        .iter()
        .filter(|&&t| t > now_ms - BUFFER_MS && t <= now_ms)
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
    fn small_chunk_is_one_part() {
        let body = "a\nb\n";
        assert_eq!(split_chunk(body, 100), vec![body.to_string()]);
    }

    #[test]
    fn big_chunk_splits_on_line_boundaries() {
        let body = "aaaa\nbbbb\ncccc\n";
        let parts = split_chunk(body, 10);
        assert_eq!(parts, vec!["aaaa\nbbbb\n", "cccc\n"]);
        assert!(parts.iter().all(|p| p.len() <= 10));
    }

    #[test]
    fn oversized_line_gets_its_own_part() {
        let parts = split_chunk("aaaaaaaaaaaa\nb\n", 10);
        assert_eq!(parts, vec!["aaaaaaaaaaaa\n", "b\n"]);
    }

    #[test]
    fn part_keys() {
        assert_eq!(chunk_part_key(T0, 0), "raw/live/2026-10-09/1600.ndjson");
        assert_eq!(chunk_part_key(T0, 2), "raw/live/2026-10-09/1600-2.ndjson");
    }

    #[test]
    fn empty_buffer_is_done() {
        assert_eq!(next_buffer_action(None, T0, 0), BufferAction::Done);
    }

    #[test]
    fn finished_slot_is_flushed() {
        let now = T0 + 2 * SLOT_MS + 1_000;
        assert_eq!(
            next_buffer_action(Some(T0), now, 0),
            BufferAction::Flush(T0)
        );
    }

    #[test]
    fn flush_is_capped_per_alarm() {
        let now = T0 + 20 * SLOT_MS;
        assert_eq!(
            next_buffer_action(Some(T0), now, MAX_FLUSH),
            BufferAction::Done
        );
    }

    #[test]
    fn current_slot_never_flushed() {
        assert_eq!(
            next_buffer_action(Some(slot_of(T0 + 5)), T0 + 5, 0),
            BufferAction::Done
        );
    }

    #[test]
    fn overflow_after_24h_is_dropped_even_past_cap() {
        let now = T0 + BUFFER_MS + SLOT_MS;
        assert_eq!(
            next_buffer_action(Some(T0), now, MAX_FLUSH),
            BufferAction::Drop(T0)
        );
    }

    #[test]
    fn next_alarm_on_grid() {
        assert_eq!(
            next_alarm_ms(T0 + 3_456, POLL_MS),
            T0 + 10_000 + ALARM_OFFSET_MS
        );
        // Fired slightly late at +10 260: next is the following grid point, no drift.
        assert_eq!(
            next_alarm_ms(T0 + 10_260, POLL_MS),
            T0 + 20_000 + ALARM_OFFSET_MS
        );
        // Not yet fired at +10 249: still aims at +10 250.
        assert_eq!(
            next_alarm_ms(T0 + 10_249, POLL_MS),
            T0 + 10_000 + ALARM_OFFSET_MS
        );
        assert_eq!(
            next_alarm_ms(T0 + 1, FALLBACK_POLL_MS) % FALLBACK_POLL_MS,
            ALARM_OFFSET_MS
        );
    }

    #[test]
    fn routine_primary_poll_needs_no_save() {
        let before = IngestState::default();
        let mut after = before.clone();
        after.record(Src::Adsblol, true, T0);
        assert!(!after.needs_save(&before));
    }

    #[test]
    fn source_failure_or_metar_change_needs_save() {
        let before = IngestState::default();
        let mut failed = before.clone();
        failed.record(Src::Adsblol, false, T0);
        assert!(failed.needs_save(&before));
        let metar = IngestState {
            last_metar_ms: T0,
            ..before.clone()
        };
        assert!(metar.needs_save(&before));
        let health = IngestState {
            last_health_ms: T0,
            ..before.clone()
        };
        assert!(health.needs_save(&before));
    }

    #[test]
    fn fallback_retry_time_needs_save() {
        let mut before = IngestState::default();
        before.record(Src::Adsblol, false, T0);
        before.record(Src::Adsblol, false, T0);
        let mut after = before.clone();
        after.record(Src::Adsblol, false, T0 + RETRY_PRIMARY_MS);
        assert!(after.needs_save(&before));
    }

    #[test]
    fn health_refresh_every_15_min() {
        let s = IngestState {
            last_health_ms: T0,
            ..IngestState::default()
        };
        assert!(!s.health_due(T0 + HEALTH_MS - 1));
        assert!(s.health_due(T0 + HEALTH_MS));
    }

    #[test]
    fn old_state_json_still_loads() {
        let old = r#"{"source":"adsblol","primary_failures":0,"last_primary_try_ms":5,"last_metar_ms":7}"#;
        let s: IngestState = serde_json::from_str(old).unwrap();
        assert_eq!(s.last_health_ms, 0);
    }

    #[test]
    fn coverage_counts_windows() {
        let now = T0;
        let c = coverage(
            &[now - 1_000, now - 2_000, now - 15_000, now - BUFFER_MS],
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
