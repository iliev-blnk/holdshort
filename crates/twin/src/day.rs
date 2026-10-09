//! A whole airport-day: flights, events and runway bins.

use holdshort_core::airport::Airport;
use holdshort_core::{Metar, Obs};
use serde::Serialize;

use crate::classify::{Kind, arrival_fit, departure_fit, is_overflight, kind};
use crate::config::{Bin, bins};
use crate::detect::{asma_entry_ms, go_arounds, holds, liftoff, touchdown};
use crate::flights::{Track, mode, split};
use crate::geo::from_runway;
use crate::height::Weather;

const DAY_MS: i64 = 86_400_000;

/// One flight's verdict: the columns of `flights.parquet`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Flight {
    pub id: String,
    pub hex: String,
    pub flight: Option<String>,
    pub reg: Option<String>,
    pub ac_type: Option<String>,
    pub first_ms: i64,
    pub last_ms: i64,
    pub kind: Kind,
    pub arr_runway: Option<String>,
    pub touchdown_ms: Option<i64>,
    pub dep_runway: Option<String>,
    pub liftoff_ms: Option<i64>,
    /// None when only one runway matched (unambiguous).
    pub runway_margin_m: Option<f64>,
    pub time_low_confidence: bool,
    pub go_arounds: u32,
    pub holds: u32,
    pub holding_s: f64,
    pub asma_entry_ms: Option<i64>,
    pub asma_s: Option<f64>,
}

impl Flight {
    /// A flight with no verdicts yet.
    pub fn empty(id: &str, first_ms: i64) -> Self {
        Flight {
            id: id.into(),
            hex: String::new(),
            flight: None,
            reg: None,
            ac_type: None,
            first_ms,
            last_ms: first_ms,
            kind: Kind::Other,
            arr_runway: None,
            touchdown_ms: None,
            dep_runway: None,
            liftoff_ms: None,
            runway_margin_m: None,
            time_low_confidence: false,
            go_arounds: 0,
            holds: 0,
            holding_s: 0.0,
            asma_entry_ms: None,
            asma_s: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    AsmaEntry,
    GoAround,
    HoldStart,
    HoldEnd,
    Touchdown,
    Liftoff,
    ConfigChange,
}

/// One row of `events.parquet`, the timeline the console replays.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Event {
    pub flight_id: Option<String>,
    pub ms: i64,
    pub kind: EventKind,
    pub runway: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub h_agl_ft: Option<f64>,
    pub detail: Option<String>,
}

pub struct DayTwin {
    pub flights: Vec<Flight>,
    pub events: Vec<Event>,
    pub bins: Vec<Bin>,
}

fn event(
    f: &Flight,
    ms: i64,
    kind: EventKind,
    runway: Option<&str>,
    pos: Option<(f64, f64)>,
    h: Option<f64>,
) -> Event {
    Event {
        flight_id: Some(f.id.clone()),
        ms,
        kind,
        runway: runway.map(str::to_string),
        lat: pos.map(|p| p.0),
        lon: pos.map(|p| p.1),
        h_agl_ft: h,
        detail: None,
    }
}

/// Position of the track point closest in time to `ms`.
fn pos_at(t: &Track, ms: i64) -> Option<(f64, f64)> {
    t.points
        .iter()
        .min_by_key(|p| (p.ts_ms - ms).abs())
        .map(|p| (p.lat, p.lon))
}

/// Analyse one flight; returns its verdict and its events.
fn analyse(t: &Track, ap: &'static Airport, wx: &Weather) -> (Flight, Vec<Event>) {
    let first = &t.points[0];
    let last = t.points.last().expect("tracks are non-empty");
    let mut f = Flight::empty(&format!("{}-{}", t.hex, first.ts_ms), first.ts_ms);
    f.hex = t.hex.clone();
    f.last_ms = last.ts_ms;
    f.flight = mode(t.points.iter().map(|p| p.flight.clone()));
    f.reg = mode(t.points.iter().map(|p| p.reg.clone()));
    f.ac_type = mode(t.points.iter().map(|p| p.ac_type.clone()));

    let arr = arrival_fit(t, ap, wx);
    let dep = departure_fit(t, ap, wx);
    f.kind = kind(&arr, &dep, is_overflight(t, ap, wx));
    let mut ev = Vec::new();
    if let Some(fit) = &arr {
        let (ms, low) = touchdown(t, fit);
        f.arr_runway = Some(fit.runway.name.into());
        f.touchdown_ms = Some(ms);
        f.time_low_confidence |= low;
        let pos = from_runway(fit.runway, 300.0, 0.0);
        ev.push(event(
            &f,
            ms,
            EventKind::Touchdown,
            Some(fit.runway.name),
            Some(pos),
            Some(0.0),
        ));
        if let Some(entry) = asma_entry_ms(t, ap).filter(|e| *e < ms) {
            f.asma_entry_ms = Some(entry);
            f.asma_s = Some((ms - entry) as f64 / 1000.0);
            ev.push(event(
                &f,
                entry,
                EventKind::AsmaEntry,
                None,
                pos_at(t, entry),
                None,
            ));
        }
    }
    if let Some(fit) = &dep {
        let (ms, low) = liftoff(t, fit, wx);
        f.dep_runway = Some(fit.runway.name.into());
        f.liftoff_ms = Some(ms);
        f.time_low_confidence |= low;
        let pos = from_runway(fit.runway, 1_800.0, 0.0);
        ev.push(event(
            &f,
            ms,
            EventKind::Liftoff,
            Some(fit.runway.name),
            Some(pos),
            Some(0.0),
        ));
    }
    f.runway_margin_m = arr
        .as_ref()
        .or(dep.as_ref())
        .map(|fit| fit.margin_m)
        .filter(|m| m.is_finite());
    for ga in go_arounds(t, ap, wx) {
        f.go_arounds += 1;
        ev.push(event(
            &f,
            ga.ms,
            EventKind::GoAround,
            Some(ga.runway),
            Some((ga.lat, ga.lon)),
            Some(ga.min_h_ft),
        ));
    }
    for h in holds(t, ap) {
        f.holds += 1;
        f.holding_s += (h.end_ms - h.start_ms) as f64 / 1000.0;
        ev.push(event(
            &f,
            h.start_ms,
            EventKind::HoldStart,
            None,
            pos_at(t, h.start_ms),
            None,
        ));
        ev.push(event(
            &f,
            h.end_ms,
            EventKind::HoldEnd,
            None,
            pos_at(t, h.end_ms),
            None,
        ));
    }
    (f, ev)
}

/// Turn a day of positions (with the edge hours of the days around it) into
/// flights, events and runway bins. A flight belongs to the day of its
/// touchdown, else its liftoff, else its first point.
pub fn twin_day(
    ap: &'static Airport,
    day_start_ms: i64,
    obs: Vec<Obs>,
    metars: &[Metar],
) -> DayTwin {
    let wx = Weather::new(metars);
    let in_day = |ms: i64| ms >= day_start_ms && ms < day_start_ms + DAY_MS;
    let mut flights = Vec::new();
    let mut events = Vec::new();
    for t in split(obs) {
        let (f, ev) = analyse(&t, ap, &wx);
        if in_day(f.touchdown_ms.or(f.liftoff_ms).unwrap_or(f.first_ms)) {
            flights.push(f);
            events.extend(ev);
        }
    }
    let (bins, config_events) = bins(&flights, day_start_ms);
    events.extend(config_events);
    events.sort_by_key(|e| e.ms);
    DayTwin {
        flights,
        events,
        bins,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{approach, cruise, departure, racetrack, with_hex};
    use holdshort_core::airport::LTFM;

    const D: i64 = 1_791_417_600_000;

    fn rwy(name: &str) -> &'static holdshort_core::airport::RunwayEnd {
        LTFM.runways.iter().find(|r| r.name == name).unwrap()
    }

    #[test]
    fn midnight_flight_belongs_to_touchdown_day() {
        let pts = approach(rwy("35L"), 0.0, 10.0, D - 120_000);
        assert!(pts[0].ts_ms < D);
        let today = twin_day(&LTFM, D, pts.clone(), &[]);
        assert_eq!(today.flights.len(), 1);
        assert!(today.flights[0].touchdown_ms.unwrap() >= D);
        let yesterday = twin_day(&LTFM, D - 86_400_000, pts, &[]);
        assert!(yesterday.flights.is_empty());
    }

    #[test]
    fn twin_day_end_to_end() {
        let t0 = D + 3_600_000;
        let mut obs = with_hex(approach(rwy("35L"), 0.0, 45.0, t0), "aaaaaa");
        obs.extend(with_hex(departure(rwy("36"), t0), "bbbbbb"));
        obs.extend(with_hex(cruise(36_000, t0), "cccccc"));
        let mut held = racetrack(LTFM.lat + 12.0 / 60.0, LTFM.lon, 12_000.0, 2, t0);
        let after = held.last().unwrap().ts_ms + 60_000;
        held.extend(approach(rwy("35R"), 0.0, 10.0, after));
        obs.extend(with_hex(held, "dddddd"));

        let day = twin_day(&LTFM, D, obs, &[]);
        let by = |hex: &str| day.flights.iter().find(|f| f.hex == hex).unwrap();
        assert_eq!(day.flights.len(), 4);
        let a = by("aaaaaa");
        assert_eq!(
            (a.kind, a.arr_runway.as_deref()),
            (Kind::Arrival, Some("35L"))
        );
        let asma = (a.touchdown_ms.unwrap() - a.asma_entry_ms.unwrap()) as f64 / 1000.0;
        assert_eq!(a.asma_s, Some(asma));
        assert_eq!(a.id, format!("aaaaaa-{t0}"));
        assert_eq!(a.flight.as_deref(), Some("TST1"));
        let b = by("bbbbbb");
        assert_eq!(
            (b.kind, b.dep_runway.as_deref()),
            (Kind::Departure, Some("36"))
        );
        assert_eq!(by("cccccc").kind, Kind::Overflight);
        let d = by("dddddd");
        assert_eq!((d.kind, d.holds), (Kind::Arrival, 1));
        assert!(d.holding_s > 400.0);

        let kinds: Vec<EventKind> = day.events.iter().map(|e| e.kind).collect();
        for k in [
            EventKind::AsmaEntry,
            EventKind::Touchdown,
            EventKind::Liftoff,
            EventKind::HoldStart,
            EventKind::HoldEnd,
        ] {
            assert!(kinds.contains(&k), "{k:?} missing");
        }
        assert!(day.events.windows(2).all(|w| w[0].ms <= w[1].ms));
        // One movement per runway is below the "active" bar: bins, but no config.
        assert!(!day.bins.is_empty());
        assert!(!kinds.contains(&EventKind::ConfigChange));
    }
}
