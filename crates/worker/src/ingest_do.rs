//! The `Ingest` Durable Object: one instance (`ltfm`) that polls the feeds on
//! an alarm loop, buffers each poll as one SQLite row, and flushes finished
//! 5-min slots to D1.
//!
//! Free-plan budget (per day: 100k rows written, 5M rows read). Per alarm:
//! one `buf` insert, one `polls` insert (ok polls only), state saved only when
//! it changes. Per 5-min flush: read and delete one slot's ~30 rows. Every
//! 15 min: one scan of `polls` for coverage plus pruning. No indexes, because
//! each index update counts as another row written. That is about 35k writes
//! and 2M reads a day.

use std::time::Duration;

use chrono::DateTime;
use holdshort_core::area::retain_in_area;
use holdshort_core::ingest::{
    BUFFER_MS, BufferAction, Coverage, IngestState, MAX_PART_BYTES, chunk_part_key, coverage,
    next_alarm_ms, next_buffer_action, slot_of, split_chunk,
};
use holdshort_core::{Line, Obs, Poll, Src, adsblol, metar, opensky, to_ndjson};
use serde::{Deserialize, Serialize};
use worker::wasm_bindgen::JsValue;
use worker::*;

use crate::http;

const ADSBLOL_URL: &str = "https://api.adsb.lol/v2/point/41.2753/28.7519/50";
const AWC_URL: &str = "https://aviationweather.gov/api/data/metar?ids=LTFM&format=json&hours=1";
const ADSBLOL_TIMEOUT_MS: u32 = 5_000;
const OTHER_TIMEOUT_MS: u32 = 10_000;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS buf (slot INTEGER NOT NULL, chunk TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS polls (ts_ms INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v TEXT NOT NULL);
";

// `#[durable_object]` + `impl DurableObject` generate the JS class the
// runtime instantiates; the struct is our state between events.
#[durable_object]
pub struct Ingest {
    state: State,
    env: Env,
    sql: SqlStorage,
}

#[derive(Serialize)]
struct Health {
    source: Src,
    last_ok_ts: Option<i64>,
    buffered_rows: i64,
    windows_24h: Option<CachedCoverage>,
    last_error: Option<LastError>,
}

#[derive(Serialize, Deserialize)]
struct CachedCoverage {
    as_of_ms: i64,
    #[serde(flatten)]
    coverage: Coverage,
}

#[derive(Serialize, Deserialize)]
struct LastError {
    ts_ms: i64,
    src: Src,
    error: String,
}

#[derive(Deserialize)]
struct Ts {
    ts_ms: i64,
}

#[derive(Deserialize)]
struct Count {
    n: i64,
}

#[derive(Deserialize)]
struct SlotRow {
    slot: i64,
}

#[derive(Deserialize)]
struct ChunkRow {
    chunk: String,
}

#[derive(Deserialize)]
struct MetaRow {
    v: String,
}

#[derive(Serialize, Deserialize)]
struct Token {
    token: String,
    exp_ms: i64,
}

fn now_ms() -> i64 {
    Date::now().as_millis() as i64
}

impl DurableObject for Ingest {
    fn new(state: State, env: Env) -> Self {
        let sql = state.storage().sql();
        sql.exec(SCHEMA, None).expect("create schema");
        Self { state, env, sql }
    }

    async fn fetch(&self, req: Request) -> Result<Response> {
        match (req.method(), req.path().as_str()) {
            (Method::Get, "/health") => Response::from_json(&self.health()?),
            (Method::Post, "/ensure") => {
                let armed = self.ensure().await?;
                Response::from_json(&serde_json::json!({ "armed": armed }))
            }
            _ => Response::error("Not found", 404),
        }
    }

    async fn alarm(&self) -> Result<Response> {
        let now = now_ms();
        let before = self.load_state();
        let mut st = before.clone();
        // Arm first, on a fixed grid, so a failure below never stops the loop
        // and the handler's run time never makes it drift.
        self.arm(next_alarm_ms(now, st.interval_ms())).await?;
        // Other requests (e.g. /health) can run while this awaits a fetch;
        // only storage operations are serialised.
        let mut lines = self.poll(&mut st, now).await?;
        if st.metar_due(now) {
            st.last_metar_ms = now;
            lines.extend(self.poll_metar().await);
        }
        self.buffer(slot_of(now), &lines)?;
        self.drain_buffer(now, st.source).await?;
        if st.health_due(now) {
            st.last_health_ms = now;
            self.refresh_coverage(now)?;
        }
        if st.needs_save(&before) {
            self.set_meta("state", &serde_json::to_string(&st)?)?;
        }
        if st.interval_ms() != before.interval_ms() {
            self.arm(next_alarm_ms(now, st.interval_ms())).await?;
        }
        Response::ok("ok")
    }
}

impl Ingest {
    /// Set an alarm for now if none is set. Returns true if one was set.
    async fn ensure(&self) -> Result<bool> {
        let storage = self.state.storage();
        if storage.get_alarm().await?.is_some() {
            return Ok(false);
        }
        storage.set_alarm(Duration::ZERO).await?;
        Ok(true)
    }

    /// Schedule the next alarm at an absolute time (ms since epoch).
    async fn arm(&self, at_ms: i64) -> Result<()> {
        let at = DateTime::from_timestamp_millis(at_ms).expect("alarm time in range");
        self.state.storage().set_alarm(at).await
    }

    /// Poll the chosen feed once. Returns the NDJSON lines for this poll
    /// (positions + one status line) and records the outcome.
    async fn poll(&self, st: &mut IngestState, now: i64) -> Result<Vec<Line>> {
        let src = st.choose(now);
        let result = match src {
            Src::Adsblol => http::get_text(ADSBLOL_URL, &[], ADSBLOL_TIMEOUT_MS)
                .await
                .and_then(|b| adsblol::parse(&b, now).map_err(|e| e.to_string())),
            _ => self.poll_opensky(now).await,
        };
        let latency_ms = (now_ms() - now) as u32;
        let mut lines = Vec::new();
        let poll = match result {
            Ok(mut obs) => {
                retain_in_area(&mut obs);
                let count = obs.len() as u32;
                lines.extend(obs.into_iter().map(Line::Obs));
                self.sql
                    .exec("INSERT INTO polls (ts_ms) VALUES (?)", vec![now.into()])?;
                Poll {
                    ts_ms: now,
                    src,
                    ok: true,
                    count,
                    latency_ms,
                    error: None,
                }
            }
            Err(e) => {
                console_error!("{src:?} poll failed: {e}");
                let last = LastError {
                    ts_ms: now,
                    src,
                    error: e.clone(),
                };
                self.set_meta("last_error", &serde_json::to_string(&last)?)?;
                Poll {
                    ts_ms: now,
                    src,
                    ok: false,
                    count: 0,
                    latency_ms,
                    error: Some(e),
                }
            }
        };
        st.record(src, poll.ok, now);
        lines.push(Line::Poll(poll));
        Ok(lines)
    }

    async fn poll_opensky(&self, now: i64) -> std::result::Result<Vec<Obs>, String> {
        let (lamin, lomin, lamax, lomax) = opensky::BBOX;
        let url = format!(
            "https://opensky-network.org/api/states/all?lamin={lamin}&lomin={lomin}&lamax={lamax}&lomax={lomax}"
        );
        let body = match self.opensky_token(now).await {
            Some(t) => {
                let auth = format!("Bearer {t}");
                http::get_text(&url, &[("Authorization", &auth)], OTHER_TIMEOUT_MS).await?
            }
            None => http::get_text(&url, &[], OTHER_TIMEOUT_MS).await?,
        };
        opensky::parse(&body, now).map_err(|e| e.to_string())
    }

    /// Cached OpenSky token; refreshed a minute before it expires.
    async fn opensky_token(&self, now: i64) -> Option<String> {
        let cached = self
            .meta("opensky_token")
            .and_then(|v| serde_json::from_str::<Token>(&v).ok())
            .filter(|t| t.exp_ms - 60_000 > now);
        if let Some(t) = cached {
            return Some(t.token);
        }
        match http::fetch_opensky_token(&self.env).await {
            Ok(Some((token, expires_in))) => {
                let t = Token {
                    token,
                    exp_ms: now + expires_in * 1000,
                };
                let _ = self.set_meta("opensky_token", &serde_json::to_string(&t).ok()?);
                Some(t.token)
            }
            Ok(None) => None,
            Err(e) => {
                console_error!("opensky token failed: {e}");
                None
            }
        }
    }

    async fn poll_metar(&self) -> Vec<Line> {
        let parsed = http::get_text(AWC_URL, &[], OTHER_TIMEOUT_MS)
            .await
            .and_then(|b| metar::parse_awc(&b).map_err(|e| e.to_string()));
        match parsed {
            Ok(ms) => ms.into_iter().map(Line::Metar).collect(),
            Err(e) => {
                console_error!("metar failed: {e}");
                Vec::new()
            }
        }
    }

    /// Buffer one poll's lines as a single row.
    fn buffer(&self, slot: i64, lines: &[Line]) -> Result<()> {
        self.sql.exec(
            "INSERT INTO buf (slot, chunk) VALUES (?, ?)",
            vec![slot.into(), to_ndjson(lines).into()],
        )?;
        Ok(())
    }

    /// Oldest buffered slot. Rows are inserted in time order, so the first
    /// rowid is the oldest: one row read, no index needed.
    fn oldest_slot(&self) -> Result<Option<i64>> {
        let rows: Vec<SlotRow> = self
            .sql
            .exec("SELECT slot FROM buf ORDER BY rowid LIMIT 1", None)?
            .to_array()?;
        Ok(rows.first().map(|r| r.slot))
    }

    /// Drop slots past the 24 h cap (recording a gap) and flush finished
    /// slots to D1, at most `MAX_FLUSH` per alarm.
    async fn drain_buffer(&self, now: i64, src: Src) -> Result<()> {
        let mut flushed = 0;
        loop {
            match next_buffer_action(self.oldest_slot()?, now, flushed) {
                BufferAction::Drop(slot) => {
                    self.sql
                        .exec("DELETE FROM buf WHERE slot = ?", vec![slot.into()])?;
                    let gap = Poll {
                        ts_ms: slot,
                        src,
                        ok: false,
                        count: 0,
                        latency_ms: 0,
                        error: Some("buffer_overflow".into()),
                    };
                    self.buffer(slot_of(now), &[Line::Poll(gap)])?;
                }
                BufferAction::Flush(slot) => {
                    if let Err(e) = self.flush_slot(slot, now).await {
                        console_error!("flush {slot} failed: {e}");
                        return Ok(());
                    }
                    flushed += 1;
                }
                BufferAction::Done => return Ok(()),
            }
        }
    }

    async fn flush_slot(&self, slot: i64, now: i64) -> Result<()> {
        let rows: Vec<ChunkRow> = self
            .sql
            .exec(
                "SELECT chunk FROM buf WHERE slot = ? ORDER BY rowid",
                vec![slot.into()],
            )?
            .to_array()?;
        let body: String = rows.into_iter().map(|r| r.chunk).collect();
        let db = self.env.d1("DB")?;
        // Insert parts as one batch so a slot lands in D1 completely or not at all.
        let mut stmts = Vec::new();
        for (i, part) in split_chunk(&body, MAX_PART_BYTES).into_iter().enumerate() {
            stmts.push(
                db.prepare(
                    "INSERT OR REPLACE INTO chunks (key, body, created_ms) VALUES (?, ?, ?)",
                )
                .bind(&[
                    JsValue::from_str(&chunk_part_key(slot, i)),
                    JsValue::from_str(&part),
                    JsValue::from_f64(now as f64),
                ])?,
            );
        }
        db.batch(stmts).await?;
        self.sql
            .exec("DELETE FROM buf WHERE slot = ?", vec![slot.into()])?;
        Ok(())
    }

    /// Recompute 24 h coverage (one scan of `polls`) and prune old polls.
    fn refresh_coverage(&self, now: i64) -> Result<()> {
        self.sql.exec(
            "DELETE FROM polls WHERE ts_ms <= ?",
            vec![(now - BUFFER_MS).into()],
        )?;
        let rows: Vec<Ts> = self.sql.exec("SELECT ts_ms FROM polls", None)?.to_array()?;
        let ok: Vec<i64> = rows.into_iter().map(|r| r.ts_ms).collect();
        let cached = CachedCoverage {
            as_of_ms: now,
            coverage: coverage(&ok, now),
        };
        self.set_meta("coverage", &serde_json::to_string(&cached)?)
    }

    fn meta(&self, k: &str) -> Option<String> {
        let rows: Vec<MetaRow> = self
            .sql
            .exec("SELECT v FROM meta WHERE k = ?", vec![k.into()])
            .ok()?
            .to_array()
            .ok()?;
        rows.into_iter().next().map(|r| r.v)
    }

    /// A meta value decoded from JSON. Generic over `T`, which a closure can't be.
    fn meta_json<T: serde::de::DeserializeOwned>(&self, k: &str) -> Option<T> {
        self.meta(k).and_then(|v| serde_json::from_str(&v).ok())
    }

    fn set_meta(&self, k: &str, v: &str) -> Result<()> {
        self.sql.exec(
            "INSERT OR REPLACE INTO meta (k, v) VALUES (?, ?)",
            vec![k.into(), v.into()],
        )?;
        Ok(())
    }

    fn load_state(&self) -> IngestState {
        self.meta_json("state").unwrap_or_default()
    }

    /// Cheap by design: a few meta rows, the last poll, and the buffer count.
    fn health(&self) -> Result<Health> {
        let last_ok: Vec<Ts> = self
            .sql
            .exec("SELECT ts_ms FROM polls ORDER BY rowid DESC LIMIT 1", None)?
            .to_array()?;
        let buffered: Count = self
            .sql
            .exec("SELECT COUNT(*) AS n FROM buf", None)?
            .one()?;
        Ok(Health {
            source: self.load_state().source,
            last_ok_ts: last_ok.first().map(|r| r.ts_ms),
            buffered_rows: buffered.n,
            windows_24h: self.meta_json("coverage"),
            last_error: self.meta_json("last_error"),
        })
    }
}
