//! Cloudflare D1 over the REST API: read live chunks the worker flushed.

use anyhow::{Context, anyhow};
use serde::Deserialize;
use serde_json::json;

use crate::UA;

/// The `holdshort` D1 database (same id as crates/worker/wrangler.toml).
pub const DATABASE_ID: &str = "6b01499f-78e6-4af7-aaab-4e9faab2905d";

#[derive(Deserialize)]
struct ApiResponse {
    success: bool,
    #[serde(default)]
    errors: Vec<ApiError>,
    result: Option<Vec<QueryResult>>,
}

#[derive(Deserialize)]
struct ApiError {
    message: String,
}

#[derive(Deserialize)]
struct QueryResult {
    results: Vec<ChunkRow>,
}

#[derive(Deserialize)]
struct ChunkRow {
    key: String,
    body: String,
}

/// `(key, body)` rows from a D1 `/query` response.
pub fn parse_chunk_rows(json: &str) -> anyhow::Result<Vec<(String, String)>> {
    let resp: ApiResponse = serde_json::from_str(json)?;
    if !resp.success {
        let msgs: Vec<String> = resp.errors.into_iter().map(|e| e.message).collect();
        return Err(anyhow!("D1 query failed: {}", msgs.join("; ")));
    }
    Ok(resp
        .result
        .unwrap_or_default()
        .into_iter()
        .flat_map(|r| r.results)
        .map(|r| (r.key, r.body))
        .collect())
}

/// Primary-key range covering every chunk part of one UTC hour. `~` sorts
/// after digits, `-` and `.`, so this is an index range scan, not a table scan.
pub fn hour_range(date: &str, hour: u32) -> (String, String) {
    let from = format!("raw/live/{date}/{hour:02}");
    let to = format!("{from}~");
    (from, to)
}

pub struct D1 {
    client: reqwest::blocking::Client,
    url: String,
    token: String,
}

impl D1 {
    /// From `CLOUDFLARE_ACCOUNT_ID` and `CLOUDFLARE_API_TOKEN` (D1 read).
    pub fn from_env() -> anyhow::Result<Self> {
        let var = |n: &str| std::env::var(n).with_context(|| format!("missing env var {n}"));
        Ok(Self {
            client: reqwest::blocking::Client::builder()
                .user_agent(UA)
                .build()?,
            url: format!(
                "https://api.cloudflare.com/client/v4/accounts/{}/d1/database/{DATABASE_ID}/query",
                var("CLOUDFLARE_ACCOUNT_ID")?
            ),
            token: var("CLOUDFLARE_API_TOKEN")?,
        })
    }

    /// Chunks with `from <= key < to`.
    pub fn chunks_between(&self, from: &str, to: &str) -> anyhow::Result<Vec<(String, String)>> {
        let body = json!({
            "sql": "SELECT key, body FROM chunks WHERE key >= ? AND key < ? ORDER BY key",
            "params": [from, to],
        });
        let text = self
            .client
            .post(&self.url)
            .bearer_auth(&self.token)
            .json(&body)
            .send()?
            .text()?;
        parse_chunk_rows(&text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_query_rows() {
        let json = r#"{"result":[{"results":[{"key":"raw/live/2026-10-09/1600.ndjson","body":"x\n"}],
            "success":true,"meta":{}}],"success":true,"errors":[],"messages":[]}"#;
        let rows = parse_chunk_rows(json).unwrap();
        assert_eq!(
            rows,
            vec![(
                "raw/live/2026-10-09/1600.ndjson".to_string(),
                "x\n".to_string()
            )]
        );
    }

    #[test]
    fn api_error_is_error() {
        let json = r#"{"result":null,"success":false,"errors":[{"code":7403,"message":"not authorized"}]}"#;
        let err = parse_chunk_rows(json).unwrap_err().to_string();
        assert!(err.contains("not authorized"), "{err}");
    }

    #[test]
    fn hour_range_is_tight() {
        let (from, to) = hour_range("2026-10-09", 7);
        assert_eq!(from, "raw/live/2026-10-09/07");
        assert_eq!(to, "raw/live/2026-10-09/07~");
        assert!("raw/live/2026-10-09/0755-1.ndjson" < to.as_str());
        assert!("raw/live/2026-10-09/0800.ndjson" > to.as_str());
    }
}
