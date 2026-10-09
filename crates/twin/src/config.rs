//! Runway use in 10-minute bins and the runway configuration over a day.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::day::{Event, EventKind, Flight};

pub const BIN_MS: i64 = 600_000;
const BINS_PER_DAY: i64 = 144;
const ACTIVE_MS: i64 = 1_800_000;
/// Movements of one kind a runway needs in the last 30 min to count as active.
const ACTIVE_MIN_MOVES: usize = 2;
/// Bins a configuration must hold to count.
const STABLE_BINS: usize = 2;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Bin {
    pub bin_ms: i64,
    pub runway: String,
    pub arrivals: u32,
    pub departures: u32,
    /// Configuration label at the end of the bin.
    pub config: String,
}

type Sets = (BTreeSet<String>, BTreeSet<String>);

fn label((arr, dep): &Sets) -> String {
    let join = |s: &BTreeSet<String>| {
        if s.is_empty() {
            "-".to_string()
        } else {
            s.iter().cloned().collect::<Vec<_>>().join(",")
        }
    };
    format!("ARR {} / DEP {}", join(arr), join(dep))
}

/// Runways with at least two movements of each kind in `(end − 30 min, end]`.
fn active(moves: &[(i64, String, bool)], end: i64) -> Sets {
    let mut count: BTreeMap<(&str, bool), usize> = BTreeMap::new();
    for (ms, rwy, is_arr) in moves {
        if *ms > end - ACTIVE_MS && *ms <= end {
            *count.entry((rwy.as_str(), *is_arr)).or_default() += 1;
        }
    }
    let pick = |want: bool| {
        count
            .iter()
            .filter(|((_, a), n)| *a == want && **n >= ACTIVE_MIN_MOVES)
            .map(|((r, _), _)| r.to_string())
            .collect()
    };
    (pick(true), pick(false))
}

/// 10-minute bins per runway (for runways used during the day) and a
/// config-change event at the start of each stable configuration.
pub fn bins(flights: &[Flight], day_start_ms: i64) -> (Vec<Bin>, Vec<Event>) {
    let mut moves: Vec<(i64, String, bool)> = Vec::new();
    for f in flights {
        if let (Some(ms), Some(r)) = (f.touchdown_ms, &f.arr_runway) {
            moves.push((ms, r.clone(), true));
        }
        if let (Some(ms), Some(r)) = (f.liftoff_ms, &f.dep_runway) {
            moves.push((ms, r.clone(), false));
        }
    }
    let runways: BTreeSet<String> = moves.iter().map(|m| m.1.clone()).collect();

    let mut out = Vec::new();
    let mut sets: Vec<Sets> = Vec::new();
    for k in 0..BINS_PER_DAY {
        let start = day_start_ms + k * BIN_MS;
        let end = start + BIN_MS;
        let s = active(&moves, end);
        let config = label(&s);
        for r in &runways {
            let in_bin = |want: bool| {
                moves
                    .iter()
                    .filter(|(ms, rr, a)| *ms >= start && *ms < end && rr == r && *a == want)
                    .count() as u32
            };
            out.push(Bin {
                bin_ms: start,
                runway: r.clone(),
                arrivals: in_bin(true),
                departures: in_bin(false),
                config: config.clone(),
            });
        }
        sets.push(s);
    }

    // Stable segments: a non-empty configuration held for at least 2 bins.
    let mut segs: Vec<(usize, Sets)> = Vec::new();
    let mut k = 0;
    while k < sets.len() {
        let mut j = k;
        while j + 1 < sets.len() && sets[j + 1] == sets[k] {
            j += 1;
        }
        let empty = sets[k].0.is_empty() && sets[k].1.is_empty();
        if !empty && j + 1 - k >= STABLE_BINS {
            segs.push((k, sets[k].clone()));
        }
        k = j + 1;
    }
    // Drop transitions: a segment that is just its neighbours' runways together.
    let union = |a: &Sets, b: &Sets| -> Sets { (&a.0 | &b.0, &a.1 | &b.1) };
    let mut kept: Vec<(usize, Sets)> = Vec::new();
    for i in 0..segs.len() {
        let transition =
            i > 0 && i + 1 < segs.len() && segs[i].1 == union(&segs[i - 1].1, &segs[i + 1].1);
        if !transition && kept.last().is_none_or(|l| l.1 != segs[i].1) {
            kept.push(segs[i].clone());
        }
    }
    let events = kept
        .into_iter()
        .map(|(k, s)| Event {
            flight_id: None,
            ms: day_start_ms + k as i64 * BIN_MS,
            kind: EventKind::ConfigChange,
            runway: None,
            lat: None,
            lon: None,
            h_agl_ft: None,
            detail: Some(label(&s)),
        })
        .collect();
    (out, events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::Kind;

    const D: i64 = 1_791_417_600_000; // 2026-10-08 00:00 UTC
    const MIN: i64 = 60_000;

    fn arr(rwy: &str, ms: i64) -> Flight {
        let mut f = Flight::empty(&format!("a{ms}"), ms);
        f.kind = Kind::Arrival;
        f.arr_runway = Some(rwy.into());
        f.touchdown_ms = Some(ms);
        f
    }

    fn dep(rwy: &str, ms: i64) -> Flight {
        let mut f = Flight::empty(&format!("d{ms}"), ms);
        f.kind = Kind::Departure;
        f.dep_runway = Some(rwy.into());
        f.liftoff_ms = Some(ms);
        f
    }

    /// One movement every 2 minutes per role, alternating runways.
    fn traffic(from: i64, to: i64, arrs: [&str; 2], deps: [&str; 2]) -> Vec<Flight> {
        let mut v = Vec::new();
        let mut t = from;
        let mut k = 0;
        while t < to {
            v.push(arr(arrs[k % 2], t));
            v.push(dep(deps[k % 2], t + MIN));
            t += 2 * MIN;
            k += 1;
        }
        v
    }

    fn changes(events: &[Event]) -> Vec<String> {
        events
            .iter()
            .filter(|e| e.kind == EventKind::ConfigChange)
            .map(|e| e.detail.clone().unwrap())
            .collect()
    }

    #[test]
    fn config_label_and_change() {
        let mut fl = traffic(D, D + 60 * MIN, ["35L", "35R"], ["34L", "36"]);
        fl.extend(traffic(
            D + 60 * MIN,
            D + 120 * MIN,
            ["16R", "17L"],
            ["16L", "18"],
        ));
        let (bins, events) = bins(&fl, D);
        assert_eq!(
            changes(&events),
            vec!["ARR 35L,35R / DEP 34L,36", "ARR 16R,17L / DEP 16L,18"]
        );
        let first = bins
            .iter()
            .find(|b| b.bin_ms == D && b.runway == "35L")
            .unwrap();
        assert_eq!((first.arrivals, first.departures), (3, 0));
        assert_eq!(bins.len(), 144 * 8);
    }

    #[test]
    fn adding_a_runway_is_a_change() {
        let mut fl = traffic(D, D + 60 * MIN, ["35L", "35R"], ["34L", "36"]);
        fl.extend(traffic(
            D + 60 * MIN,
            D + 180 * MIN,
            ["35L", "35R"],
            ["34L", "36"],
        ));
        fl.extend((0..30).map(|k| arr("34R", D + 60 * MIN + k * 2 * MIN)));
        let (_, events) = bins(&fl, D);
        assert_eq!(
            changes(&events),
            vec![
                "ARR 35L,35R / DEP 34L,36",
                "ARR 34R,35L,35R / DEP 34L,36",
                "ARR 35L,35R / DEP 34L,36"
            ]
        );
    }

    #[test]
    fn flapping_is_suppressed() {
        let mut fl = traffic(D, D + 120 * MIN, ["35L", "35R"], ["34L", "36"]);
        // One stray arrival on 16R: active for 30 min = 3 bins, but as a
        // union with the same config on both sides it is dropped.
        fl.push(arr("16R", D + 50 * MIN));
        let (_, events) = bins(&fl, D);
        assert_eq!(changes(&events), vec!["ARR 35L,35R / DEP 34L,36"]);
    }
}
