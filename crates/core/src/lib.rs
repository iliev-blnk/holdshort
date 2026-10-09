//! Pure ingest logic shared by the Cloudflare worker and the native CLI.
//! No I/O here, so it builds for both native and wasm32.

pub mod area;
pub mod model;

pub use model::*;

/// Errors from parsing feeds and NDJSON.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("format: {0}")]
    Format(String),
}
