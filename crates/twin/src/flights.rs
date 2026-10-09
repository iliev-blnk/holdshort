//! Cut a day of positions into flights.

use std::collections::{BTreeMap, HashMap};

use holdshort_core::Obs;

/// A gap longer than this between two points of one aircraft starts a new
/// flight. Landed aircraft vanish (no ground coverage) for longer than this.
pub const GAP_MS: i64 = 1_200_000;

/// One flight: consecutive positions of one aircraft, oldest first.
#[derive(Debug, Clone)]
pub struct Track {
    pub hex: String,
    pub points: Vec<Obs>,
}

/// Group by aircraft, sort by time, and split on gaps over 20 minutes.
pub fn split(obs: Vec<Obs>) -> Vec<Track> {
    // A BTreeMap keeps the output order stable (by hex).
    let mut by_hex: BTreeMap<String, Vec<Obs>> = BTreeMap::new();
    for o in obs {
        by_hex.entry(o.hex.clone()).or_default().push(o);
    }
    let mut tracks = Vec::new();
    for (hex, mut pts) in by_hex {
        pts.sort_by_key(|o| o.ts_ms);
        let mut cur: Vec<Obs> = Vec::new();
        for o in pts {
            if cur.last().is_some_and(|p| o.ts_ms - p.ts_ms > GAP_MS) {
                tracks.push(Track {
                    hex: hex.clone(),
                    points: std::mem::take(&mut cur),
                });
            }
            cur.push(o);
        }
        tracks.push(Track { hex, points: cur });
    }
    tracks
}

/// Most frequent non-empty value; ties go to the first seen.
pub fn mode(values: impl Iterator<Item = Option<String>>) -> Option<String> {
    let mut counts: HashMap<String, (usize, usize)> = HashMap::new();
    for (i, v) in values.flatten().filter(|v| !v.is_empty()).enumerate() {
        counts.entry(v).or_insert((0, i)).0 += 1;
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.0.cmp(&b.1.0).then(b.1.1.cmp(&a.1.1)))
        .map(|(v, _)| v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{cruise, with_hex};

    #[test]
    fn gap_over_20_min_splits() {
        let mut pts = with_hex(cruise(36_000, 0), "aaaaaa");
        pts.truncate(3);
        let mut late = pts[2].clone();
        late.ts_ms += 25 * 60_000;
        pts.push(late);
        let tracks = split(pts);
        assert_eq!(tracks.len(), 2);
        assert_eq!((tracks[0].points.len(), tracks[1].points.len()), (3, 1));
    }

    #[test]
    fn hexes_are_separate() {
        let mut pts = with_hex(cruise(36_000, 0), "aaaaaa");
        pts.extend(with_hex(cruise(30_000, 0), "bbbbbb"));
        let tracks = split(pts);
        assert_eq!(tracks.len(), 2);
        assert!(
            tracks
                .iter()
                .all(|t| t.points.iter().all(|p| p.hex == t.hex))
        );
    }

    #[test]
    fn unsorted_input_is_sorted() {
        let mut pts = cruise(36_000, 0);
        pts.reverse();
        let t = &split(pts)[0];
        assert!(t.points.windows(2).all(|w| w[0].ts_ms <= w[1].ts_ms));
    }

    #[test]
    fn mode_picks_most_frequent() {
        let v = [None, Some("A"), Some("B"), Some("B"), Some("")];
        assert_eq!(
            mode(v.iter().map(|x| x.map(str::to_string))),
            Some("B".into())
        );
        assert_eq!(mode(std::iter::empty()), None);
    }
}
