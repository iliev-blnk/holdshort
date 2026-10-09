//! `holdshort twin-check`: sanity report over real days, plus a sample of
//! flights to review by hand.

use chrono::DateTime;
use holdshort_core::Obs;
use holdshort_core::airport::Airport;
use holdshort_twin::classify::AMBIGUOUS_M;
use holdshort_twin::day::Flight;
use holdshort_twin::height::Weather;

const MAX_TAILWIND_KT: f64 = 5.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub days: usize,
    pub movements_per_day: f64,
    pub runway_ok_pct: f64,
    pub into_wind_pct: f64,
    pub go_arounds_per_1000: f64,
    pub median_asma_s: Option<f64>,
}

/// Wind component along the landing direction, positive = from behind.
pub fn tailwind_kt(wind_from_deg: f64, wind_kt: f64, runway_heading_deg: f64) -> f64 {
    -wind_kt * (wind_from_deg - runway_heading_deg).to_radians().cos()
}

pub fn median(v: &mut [f64]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let m = v.len() / 2;
    Some(if v.len() % 2 == 1 {
        v[m]
    } else {
        (v[m - 1] + v[m]) / 2.0
    })
}

fn pct(n: usize, of: usize) -> f64 {
    if of == 0 {
        0.0
    } else {
        n as f64 / of as f64 * 100.0
    }
}

/// The five sanity measures over `days` days of flights.
pub fn evaluate(flights: &[Flight], wx: &Weather, ap: &Airport, days: usize) -> Report {
    let arrivals: Vec<&Flight> = flights
        .iter()
        .filter(|f| f.touchdown_ms.is_some())
        .collect();
    let departures = flights.iter().filter(|f| f.liftoff_ms.is_some()).count();
    let clear = arrivals
        .iter()
        .filter(|f| f.runway_margin_m.is_none_or(|m| m >= AMBIGUOUS_M))
        .count();
    let (mut with_wind, mut into_wind) = (0, 0);
    for f in &arrivals {
        let heading = ap
            .runways
            .iter()
            .find(|r| Some(r.name) == f.arr_runway.as_deref())
            .map(|r| r.heading_deg);
        let Some(w) = f.touchdown_ms.and_then(|t| wx.at(t)) else {
            continue;
        };
        let Some(kt) = w.wind_kt else { continue };
        with_wind += 1;
        // Calm or variable wind counts as fine.
        let ok = match (w.wind_dir_deg, heading) {
            (Some(dir), Some(h)) => tailwind_kt(dir, kt, h) <= MAX_TAILWIND_KT,
            _ => true,
        };
        into_wind += ok as usize;
    }
    let gas: u32 = flights.iter().map(|f| f.go_arounds).sum();
    let mut asma: Vec<f64> = flights.iter().filter_map(|f| f.asma_s).collect();
    Report {
        days,
        movements_per_day: (arrivals.len() + departures) as f64 / days.max(1) as f64,
        runway_ok_pct: pct(clear, arrivals.len()),
        into_wind_pct: pct(into_wind, with_wind),
        go_arounds_per_1000: if arrivals.is_empty() {
            0.0
        } else {
            gas as f64 / arrivals.len() as f64 * 1000.0
        },
        median_asma_s: median(&mut asma),
    }
}

/// The spec's pass/fail checks, in order, with a readable line each.
pub fn thresholds(r: &Report, dhmi_per_day: f64) -> Vec<(String, bool)> {
    let dev = (r.movements_per_day - dhmi_per_day) / dhmi_per_day * 100.0;
    let asma = r.median_asma_s.unwrap_or(f64::NAN);
    vec![
        (
            format!(
                "movements/day {:.0} vs DHMI {dhmi_per_day:.0} ({dev:+.1}%, within ±15%)",
                r.movements_per_day
            ),
            dev.abs() <= 15.0 + 1e-9,
        ),
        (
            format!(
                "arrivals with a clear runway {:.1}% (≥ 95%)",
                r.runway_ok_pct
            ),
            r.runway_ok_pct >= 95.0,
        ),
        (
            format!(
                "landings with tailwind ≤ 5 kt {:.1}% (≥ 90%)",
                r.into_wind_pct
            ),
            r.into_wind_pct >= 90.0,
        ),
        (
            format!(
                "go-arounds per 1,000 arrivals {:.2} (0.5–10)",
                r.go_arounds_per_1000
            ),
            (0.5..=10.0).contains(&r.go_arounds_per_1000),
        ),
        (
            format!("median ASMA {:.0} s (300–1500)", asma),
            (300.0..=1500.0).contains(&asma),
        ),
    ]
}

fn hhmmss(ms: i64) -> String {
    DateTime::from_timestamp_millis(ms)
        .map(|t| t.format("%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

/// Print one flight's verdict and every 6th point of its track.
pub fn print_sample(f: &Flight, obs: &[Obs]) {
    println!(
        "\n{} {} {:?} arr {:?} @ {} dep {:?} @ {} margin {:?} m, go-arounds {}, holds {} ({:.0} s), asma {:?} s",
        f.id,
        f.flight.as_deref().unwrap_or("?"),
        f.kind,
        f.arr_runway,
        f.touchdown_ms.map(hhmmss).unwrap_or_default(),
        f.dep_runway,
        f.liftoff_ms.map(hhmmss).unwrap_or_default(),
        f.runway_margin_m.map(|m| m.round()),
        f.go_arounds,
        f.holds,
        f.holding_s,
        f.asma_s.map(|s| s.round()),
    );
    let pts: Vec<&Obs> = obs
        .iter()
        .filter(|o| o.hex == f.hex && o.ts_ms >= f.first_ms && o.ts_ms <= f.last_ms)
        .collect();
    for o in pts.iter().step_by(6) {
        println!(
            "    {}  {:.4},{:.4}  {:>6} ft  trk {:>3}  gs {:>3}",
            hhmmss(o.ts_ms),
            o.lat,
            o.lon,
            o.alt_baro_ft.map(|a| a.to_string()).unwrap_or("-".into()),
            o.track_deg.map(|t| t.round() as i32).unwrap_or(-1),
            o.gs_kt.map(|g| g.round() as i32).unwrap_or(-1),
        );
    }
}

/// Deterministic pseudo-random indices (xorshift) for a reproducible sample.
pub fn sample_indices(n: usize, total: usize, seed: u64) -> Vec<usize> {
    let mut x = seed.max(1);
    let mut picked = Vec::new();
    while picked.len() < n.min(total) {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let i = (x % total as u64) as usize;
        if !picked.contains(&i) {
            picked.push(i);
        }
    }
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tailwind_math() {
        assert!((tailwind_kt(170.0, 10.0, 354.0) - 9.976).abs() < 0.01);
        assert!((tailwind_kt(354.0, 10.0, 354.0) + 10.0).abs() < 1e-9);
        assert!(tailwind_kt(84.0, 10.0, 354.0).abs() < 1e-9);
    }

    #[test]
    fn median_odd_and_even() {
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&mut [4.0, 1.0, 2.0, 3.0]), Some(2.5));
        assert_eq!(median(&mut []), None);
    }

    fn report() -> Report {
        Report {
            days: 10,
            movements_per_day: 1449.0,
            runway_ok_pct: 97.0,
            into_wind_pct: 95.0,
            go_arounds_per_1000: 2.0,
            median_asma_s: Some(700.0),
        }
    }

    fn all_pass(r: &Report) -> bool {
        thresholds(r, 1449.0).iter().all(|(_, ok)| *ok)
    }

    #[test]
    fn thresholds_pass_and_fail() {
        assert!(all_pass(&report()));
        let fails: [fn(&mut Report); 6] = [
            |r| r.movements_per_day = 1449.0 * 1.16,
            |r| r.runway_ok_pct = 94.9,
            |r| r.into_wind_pct = 89.9,
            |r| r.go_arounds_per_1000 = 0.4,
            |r| r.go_arounds_per_1000 = 10.1,
            |r| r.median_asma_s = Some(1501.0),
        ];
        for f in fails {
            let mut r = report();
            f(&mut r);
            assert!(!all_pass(&r), "{r:?}");
        }
        let mut edge = report();
        edge.movements_per_day = 1449.0 * 0.85;
        assert!(all_pass(&edge));
        assert_eq!(thresholds(&report(), 1449.0).len(), 5);
    }
}
