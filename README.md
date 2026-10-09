# holdshort

A digital twin of airport airspace, built from open ADS-B data. The main airport is Istanbul
(LTFM, ~1,500 flights a day); Suceava (LRSV) is a second, smaller one.

This repository is sub-project 1 of 6: **ingest**. It builds a dataset of every aircraft
position within 50 nm of each airport. Complete days come from the adsb.lol daily archive, and a live
poller adds same-day data. Each day is compacted to Parquet alongside METAR weather. Later
sub-projects build the twin engine, console and forecasting research on top of this data.

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

## Data sources and licences

- [adsb.lol](https://adsb.lol): live API and the `globe_history` daily archive. The database is
  under the Open Database License (ODbL); feeder contributions are CC0.
- [OpenSky Network](https://opensky-network.org): fallback live feed, used under its terms of use.
- [aviationweather.gov](https://aviationweather.gov): live METAR.
- [Iowa Environmental Mesonet](https://mesonet.agron.iastate.edu): METAR archive.

## Roadmap

1. **Ingest** (this repo, now)
2. Twin engine: per-aircraft state; events such as takeoff, landing, runway in use, go-around,
   holding and taxi
3. Console: map, event timeline, replay
4. Research: taxi-out time, throughput and runway-configuration forecasts compared against
   baselines, served from Rust via ONNX
5. Gate/stand allocation or anomaly detection
6. Technical write-up

## License

MIT
