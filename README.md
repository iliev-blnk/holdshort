# holdshort

A live digital twin of Istanbul Airport (LTFM) airspace, built from open ADS-B data.

This repository is sub-project 1 of 6: **ingest**. It collects every aircraft position within
50 nm of LTFM around the clock, backfills history from the adsb.lol daily archive, and compacts
each day to Parquet alongside METAR weather. Later sub-projects build the
twin engine, console and forecasting research on top of this data.

Live status: `/health` on the deployed worker (link added after deploy).

## How it works

```
 adsb.lol live API ──┐  every 10 s
 OpenSky (fallback) ─┤  every 30 s          Cloudflare Worker
 aviationweather.gov ┘  every 30 min   ┌──────────────────────────┐
                                       │ Durable Object "Ingest"  │
                                       │  alarm loop → SQLite buf │
                                       └────────────┬─────────────┘
                                                    │ 5-min NDJSON chunks
                                                    ▼
                                             D1 table "chunks"
                                                    │ holdshort pull
                                                    ▼
 adsb.lol globe_history (GitHub, ~4 GB/day) ──► local data/ ◄── Iowa Mesonet METAR
          holdshort backfill                  raw/live/…   raw/history/…
                                              parquet/date=…/obs.parquet
                                              weather/LTFM/YYYY-MM.ndjson
```

- `crates/core`: all rules as pure Rust with no I/O (record types, feed parsers, the 50 nm area
  filter, failover, chunk slots, coverage). It builds for both native and `wasm32`.
- `crates/worker`: the Cloudflare Worker. One SQLite-backed Durable Object polls on a fixed
  10 s alarm grid, buffers one row per poll, and flushes finished 5-minute slots to D1. A cron
  trigger every minute restarts the loop if the alarm was ever lost. It is built to stay inside
  the free plan: about 35k SQLite rows written and 2M read per day.
- `crates/backfill`: the `holdshort` CLI. It streams a day's history tar from GitHub, never
  writing the 4 GB to disk, keeps the LTFM area, and writes one gzip NDJSON file per day.
  `pull` copies the worker's chunks from D1; `compact` turns a day into Parquet.

Failover: after two failed adsb.lol polls the worker switches to OpenSky (30 s interval) and
retries adsb.lol every 10 minutes. Every poll writes a status row, so gaps are recorded as data
and `/health` reports the share of 10 s windows captured in the last 24 h.

## Record format

One NDJSON line per record, tagged by `k`:

```json
{"k":"obs","ts_ms":1791561731002,"hex":"4bb208","flight":"THY6VU","reg":"TC-LPH","ac_type":"A21N","lat":40.951445,"lon":28.03093,"alt_baro_ft":19550,"on_ground":false,"gs_kt":385.9,"track_deg":263.0,"vrate_fpm":1792,"src":"adsblol"}
{"k":"poll","ts_ms":1791561733502,"src":"adsblol","ok":true,"count":14,"latency_ms":156}
{"k":"metar","station":"LTFM","obs_ts_ms":1791561000000,"raw":"METAR LTFM 091550Z 20009KT CAVOK 21/06 Q1017 NOSIG"}
```

Units are aviation units: feet, knots, feet per minute.

## CLI

```sh
cargo build --release -p holdshort
export CLOUDFLARE_ACCOUNT_ID=… CLOUDFLARE_API_TOKEN=…   # D1 read, only for `pull`

holdshort metar-history 2026-09-01..2026-09-30
holdshort backfill      2026-09-01..2026-09-30   # skips days already present; --force to redo
holdshort pull          2026-10-09               # live chunks from D1
holdshort compact       2026-09-01..2026-09-30   # --data <dir> to change the data dir (default ./data)
```

## Worker

```sh
cd crates/worker
pnpm install
pnpm exec wrangler d1 migrations apply holdshort --remote
pnpm exec wrangler dev --test-scheduled   # OpenSky credentials in .dev.vars
pnpm exec wrangler deploy
```

Secrets: `OPENSKY_CLIENT_ID`, `OPENSKY_CLIENT_SECRET` (an OpenSky API client). Without them the
fallback polls OpenSky anonymously, which has a much lower daily limit.

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
