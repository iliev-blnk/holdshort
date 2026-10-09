//! The `Ingest` Durable Object: one instance (`ltfm`) that polls the feeds on
//! an alarm loop and buffers rows in its SQLite storage.

use std::time::Duration;

use holdshort_core::Src;
use holdshort_core::ingest::{Coverage, IngestState, coverage};
use serde::{Deserialize, Serialize};
use worker::*;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS buf (slot INTEGER NOT NULL, line TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS buf_slot ON buf (slot);
CREATE TABLE IF NOT EXISTS polls (ts_ms INTEGER NOT NULL, ok INTEGER NOT NULL, error TEXT);
CREATE INDEX IF NOT EXISTS polls_ts ON polls (ts_ms);
CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v TEXT NOT NULL);
";

// `#[durable_object]` + `impl DurableObject` generate the JS class the
// runtime instantiates; the struct is our state between events.
#[durable_object]
pub struct Ingest {
    state: State,
    sql: SqlStorage,
}

#[derive(Serialize)]
struct Health {
    source: Src,
    last_ok_ts: Option<i64>,
    buffered_rows: i64,
    windows_24h: Coverage,
    last_error: Option<String>,
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
struct MaybeTs {
    ts_ms: Option<i64>,
}

#[derive(Deserialize)]
struct ErrRow {
    error: Option<String>,
}

#[derive(Deserialize)]
struct MetaRow {
    v: String,
}

impl DurableObject for Ingest {
    fn new(state: State, _env: Env) -> Self {
        let sql = state.storage().sql();
        sql.exec(SCHEMA, None).expect("create schema");
        Self { state, sql }
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
        let st = self.load_state()?;
        self.arm(&st).await?;
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

    /// Schedule the next alarm. An `i64`/`Duration` alarm time is an offset from now.
    async fn arm(&self, st: &IngestState) -> Result<()> {
        let next = Duration::from_millis(st.interval_ms() as u64);
        self.state.storage().set_alarm(next).await
    }

    fn load_state(&self) -> Result<IngestState> {
        let rows: Vec<MetaRow> = self
            .sql
            .exec("SELECT v FROM meta WHERE k = 'state'", None)?
            .to_array()?;
        Ok(rows
            .first()
            .and_then(|r| serde_json::from_str(&r.v).ok())
            .unwrap_or_default())
    }

    fn health(&self) -> Result<Health> {
        let now = Date::now().as_millis() as i64;
        let ok_ts: Vec<Ts> = self
            .sql
            .exec("SELECT ts_ms FROM polls WHERE ok = 1", None)?
            .to_array()?;
        let ok_ts: Vec<i64> = ok_ts.into_iter().map(|r| r.ts_ms).collect();
        let last_ok: MaybeTs = self
            .sql
            .exec("SELECT MAX(ts_ms) AS ts_ms FROM polls WHERE ok = 1", None)?
            .one()?;
        let buffered: Count = self
            .sql
            .exec("SELECT COUNT(*) AS n FROM buf", None)?
            .one()?;
        let last_error: Vec<ErrRow> = self
            .sql
            .exec(
                "SELECT error FROM polls WHERE ok = 0 ORDER BY ts_ms DESC LIMIT 1",
                None,
            )?
            .to_array()?;
        Ok(Health {
            source: self.load_state()?.source,
            last_ok_ts: last_ok.ts_ms,
            buffered_rows: buffered.n,
            windows_24h: coverage(&ok_ts, now),
            last_error: last_error.into_iter().next().and_then(|r| r.error),
        })
    }
}
