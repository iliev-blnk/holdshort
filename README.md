# holdshort

A digital twin of airport airspace, built from open ADS-B data. The main airport is Istanbul
(LTFM, ~1,500 flights a day); Suceava (LRSV) is a second, smaller one.

This repository holds sub-projects 1–3 of 6. **Ingest** builds a dataset of every aircraft
position within 50 nm of each airport. Complete days come from the adsb.lol daily archive, and a live
poller adds same-day data. Each day is compacted to Parquet alongside METAR weather. The **twin**
turns those positions into flights and airport events: arrivals and departures with their runway
and time, go-arounds, holds, approach time (ASMA) and the runway configuration in use. Later
sub-projects build forecasting research on top of this.

**[Live console: holdshort.pages.dev](https://holdshort.pages.dev)**, a replay of any of the
10 days with every aircraft, runway and go-around.

## How it works

```
 adsb.lol globe_history (GitHub, ~4 GB/day, all feeders) ──► holdshort backfill ─┐
 adsb.lol live API (10 s) / OpenSky fallback (30 s) ───────► holdshort live ─────┤
 aviationweather.gov / Iowa Mesonet METAR ─────────────────► live, metar-history ┤
                                                                                 ▼
                                                     data/<ICAO>/raw/history/YYYY-MM-DD.ndjson.gz
                                                     data/<ICAO>/raw/live/YYYY-MM-DD/HHMM.ndjson
                                                     data/<ICAO>/weather/YYYY-MM.ndjson
                                                                                 │ holdshort compact
                                                                                 ▼
                                                     data/<ICAO>/parquet/date=YYYY-MM-DD/obs.parquet
```

- `crates/core`: all rules as pure Rust with no I/O: record types, feed parsers, the airports and
  their 50 nm circles, failover, the poll grid and coverage. It also builds for `wasm32`, so a browser console
  can reuse it.
- `crates/cli`: the `holdshort` binary.
  - `backfill` streams a day's 4 GB history tar from GitHub without writing it to disk, once per
    day however many airports are selected. Downloads resume where they stopped if the connection
    breaks. For Istanbul, one real day is about 160k positions from 1,100 aircraft in 7 minutes
    and ~40 MB of memory.
  - `live` polls on a fixed 10 s grid. After two failed adsb.lol polls it switches to OpenSky and
    retries adsb.lol every 10 minutes. Every poll writes a status line, so gaps are recorded as
    data.
  - `compact` merges a day. History wins in any hour it covers, live data fills the rest, and
    points from the next day's first chunk are kept when they belong to this day. The result is
    written as zstd Parquet.

Why the live poller runs locally rather than in the cloud: every free ADS-B API tried (adsb.lol,
airplanes.live, adsb.fi, OpenSky) rejects or rate-limits requests from Cloudflare Workers. The
first version was a Cloudflare Durable Object (see git history), and it got 429/403 responses or
timeouts on every poll. The daily history archive has no such limit and is complete, so the
dataset is built history-first.

## Record format

One NDJSON line per record, tagged by `k`:

```json
{"k":"obs","ts_ms":1791561731002,"hex":"4bb208","flight":"THY6VU","reg":"TC-LPH","ac_type":"A21N","lat":40.951445,"lon":28.03093,"alt_baro_ft":19550,"on_ground":false,"gs_kt":385.9,"track_deg":263.0,"vrate_fpm":1792,"src":"adsblol"}
{"k":"poll","ts_ms":1791561733502,"src":"adsblol","ok":true,"count":14,"latency_ms":156}
{"k":"metar","station":"LTFM","obs_ts_ms":1791561000000,"raw":"METAR LTFM 091550Z 20009KT CAVOK 21/06 Q1017 NOSIG"}
```

Units are aviation units: feet, knots, feet per minute.

## Usage

```sh
cargo build --release -p holdshort
alias holdshort=./target/release/holdshort

holdshort backfill      2026-09-01..2026-09-30 --airport LTFM --airport LRSV
holdshort metar-history 2026-09-01..2026-09-30 --airport LTFM --airport LRSV
holdshort live --airport LTFM --airport LRSV     # Ctrl-C to stop
holdshort compact       2026-09-01..2026-09-30 --airport LTFM
```

`--airport` defaults to LTFM. `backfill` skips airport-days already present (`--force` redoes
them). Data goes to `./data` (`--data <dir>` to change it). For authenticated OpenSky fallback, put an
OpenSky API client in `.env`:

```
OPENSKY_CLIENT_ID=...
OPENSKY_CLIENT_SECRET=...
```

## Twin

```sh
holdshort twin       2026-09-29..2026-10-08 --airport LTFM --airport LRSV
holdshort twin-check 2026-09-29..2026-10-08 --airport LTFM --sample 5
```

`twin` reads a day's `obs.parquet` and weather and writes `data/<ICAO>/twin/date=YYYY-MM-DD/`:

- `flights.parquet`: one row per flight, with its kind (arrival, departure, both, overflight,
  other), runway, touchdown or liftoff time, go-arounds, holds and ASMA.
- `events.parquet`: touchdown, liftoff, go_around, hold_start/hold_end, asma_entry and
  config_change, each with a time, position and height.
- `runway_bins.parquet`: arrivals and departures per runway in 10-minute bins, with the
  configuration label (for example `ARR 34L,35R / DEP -`).

The engine lives in `crates/twin`. It is pure Rust with no I/O, so the console can run it in the
browser later. A flight is an arrival when at least three points line up with a runway's extended
centreline below 2,000 ft and the last one is below 1,500 ft. A flight that coverage loses on
final also counts, if its track ends lined up and still descending below 2,500 ft within 10 nm;
its touchdown time is then marked low-confidence. The runway is the one with the
smallest mean cross-track distance. The touchdown time is extrapolated from the last point's ground
speed and its distance to the touchdown zone. Height comes from barometric altitude corrected with
the METAR QNH.

`twin-check` judges the result against public figures. For Istanbul it compares movements with
DHMI's published average (1,449 a day in April 2026) and checks that the runway, wind, go-around
and ASMA numbers are plausible.

### Results: LTFM, 10 days (29 Sep – 8 Oct 2026)

| Check | Result | Bar |
| --- | --- | --- |
| Movements per day | 1,282 (−11.5% vs DHMI) | ±15% |
| Arrivals with a clear runway | 99.9% | ≥ 95% |
| Landings with tailwind ≤ 5 kt | 92.5% | ≥ 90% |
| Go-arounds per 1,000 arrivals | 5.1 | 0.5–10 |
| Median ASMA | 16.8 min (p90 22.2) | 5–25 min |

Over the 10 days the twin found 7,111 arrivals, 5,712 departures, 36 go-arounds, 151 flights
that held (median 8.9 minutes) and 90 runway configuration changes. Every day was north flow:
arrivals landed on 35R (3,488), 34L (3,251) and 36 (334), and a few on 35L and 34R.

### Limitations

The twin can only see what the feeders see, and Istanbul's coverage has holes:

- **No ground coverage, and departures are first seen high.** Arrivals are tracked down to about
  300 ft, but departures usually appear at around 11,000 ft, 12 nm out. They are recognised by
  their climb away from the airport, but their runway is unknown (12 of 5,712 have one) and their
  liftoff time is an estimate. The configuration label therefore mostly reads `DEP -`.
- **Some movements are never seen or only seen in part.** There are about 20% fewer departures
  than arrivals, which matches departures that never come into view. Approaches to runway 36 are
  usually lost on final around 2,000 ft. The twin counts them through the lost-on-final rule
  (342 over the 10 days, 274 of them on 36) with a low-confidence touchdown time. As a check,
  83% of them depart LTFM again within 24 hours, against 86% of the other arrivals. Coverage
  also varies from day to day: from 6 to 8 October the lowest height seen on final rose from
  about 300 ft to 500–775 ft.
- **Traffic of nearby airports is excluded, not identified.** Sabiha Gökçen (SAW) and smaller
  airfields lie inside the 50 nm circle. Their traffic shows up as overflight or other.
- **Runway headings come from the thresholds.** OurAirports gives LTFM's 16/34, 17/35 and 18/36
  headings 5° off (174/354 instead of 179/359). With those headings the R runways almost never
  matched, so the twin derives each heading from the two threshold positions instead.
- **Suceava has no low-altitude coverage.** In 10 days no aircraft was seen below 14,800 ft
  within 47 nm of LRSV, so all of its 369 flights are overflights. A local feeder would fix this.
  `twin-check` reports LRSV numbers for information only.

## Console

[![The console replaying 2 October 2026 at 10:04 UTC](docs/console.png)](https://holdshort.pages.dev)

**[holdshort.pages.dev](https://holdshort.pages.dev)** replays a day at Istanbul from the twin's
output. Aircraft move on the map in their kind's colour (arrivals blue, departures orange,
other traffic grey) with two-minute trails. The timeline shows movements per 10 minutes and the
runway configuration over the day. The side panel lists events as they happen, and clicking an
aircraft or an event opens its flight. The URL keeps the day, time and selected flight
(`?d=2026-10-02&t=10:04&f=…`), so a moment can be linked.

It is a static site on Cloudflare Pages with no server:

- `holdshort export <range>` writes each day as `day.json` (flights, events, configuration
  and histogram) and `tracks.bin.gz`. The latter holds every position as little-endian columns
  that the browser reads straight into typed arrays (about 1.9 MB gzipped per day).
- `console/` is Vite + TypeScript with MapLibre and deck.gl. It interpolates positions between
  points and hides an aircraft in coverage gaps over five minutes.
- The basemap is an OpenStreetMap extract from [Protomaps](https://protomaps.com), unpacked into
  plain tile files, because Pages does not serve the byte ranges a PMTiles archive needs.

To rebuild and deploy:

```sh
holdshort export 2026-09-29..2026-10-08 --airport LTFM   # → console/public/data
console/scripts/basemap.sh                                # → console/public/basemap
pnpm --dir console install
pnpm --dir console build
pnpm --dir console run pages:deploy                       # wrangler, Cloudflare account
```

`pnpm --dir console test` runs the unit tests, and `pnpm --dir console e2e` runs a browser smoke
test on a small fixture day. Set `CHROMIUM_PATH` to use a local Chromium.

## Data sources and licences

- [adsb.lol](https://adsb.lol): live API and the `globe_history` daily archive. The database is
  under the Open Database License (ODbL); feeder contributions are CC0.
- [OpenSky Network](https://opensky-network.org): fallback live feed, used under its terms of use.
- [aviationweather.gov](https://aviationweather.gov): live METAR.
- [Protomaps](https://protomaps.com) basemap from [OpenStreetMap](https://www.openstreetmap.org/copyright) data (ODbL).
- [Iowa Environmental Mesonet](https://mesonet.agron.iastate.edu): METAR archive.

## Roadmap

1. **Ingest** (done)
2. **Twin engine** (done): flights, runways, touchdown and liftoff, go-arounds, holds, ASMA,
   runway configuration
3. **Console** (done): replay with map, timeline, events and shareable links
4. Research: forecasts of ASMA, runway configuration, hourly movements and go-around probability,
   compared against baselines and served from Rust via ONNX. Taxi times are out of reach because
   there is no ground coverage.
5. Anomaly detection
6. Technical write-up

## License

MIT
