//! Outbound HTTP with a timeout, plus the OpenSky OAuth token.

use std::time::Duration;

use futures_util::future::{Either, select};
use serde::Deserialize;
use worker::wasm_bindgen::JsValue;
use worker::*;

pub const UA: &str = "holdshort/0.1 (+https://github.com/iliev-blnk/holdshort)";
const TOKEN_URL: &str =
    "https://auth.opensky-network.org/auth/realms/opensky-network/protocol/openid-connect/token";

/// Send a request; give up after `timeout_ms`. Non-2xx is an error.
async fn send(req: Request, timeout_ms: u32) -> std::result::Result<String, String> {
    let ctrl = AbortController::default();
    let signal = ctrl.signal();
    let call = async move {
        let mut resp = Fetch::Request(req).send_with_signal(&signal).await?;
        let status = resp.status_code();
        Ok::<_, Error>((status, resp.text().await?))
    };
    let timeout = Delay::from(Duration::from_millis(timeout_ms as u64));
    // `Box::pin` gives the futures a fixed address so `select` can poll them.
    match select(Box::pin(call), timeout).await {
        Either::Left((Ok((status, body)), _)) if (200..300).contains(&status) => Ok(body),
        Either::Left((Ok((status, _)), _)) => Err(format!("http {status}")),
        Either::Left((Err(e), _)) => Err(e.to_string()),
        Either::Right(_) => {
            ctrl.abort();
            Err("timeout".into())
        }
    }
}

fn headers(extra: &[(&str, &str)]) -> Result<Headers> {
    let h = Headers::new();
    h.set("User-Agent", UA)?;
    for (k, v) in extra {
        h.set(k, v)?;
    }
    Ok(h)
}

/// GET `url` as text with the holdshort User-Agent.
pub async fn get_text(
    url: &str,
    extra: &[(&str, &str)],
    timeout_ms: u32,
) -> std::result::Result<String, String> {
    let mut init = RequestInit::new();
    init.with_method(Method::Get)
        .with_headers(headers(extra).map_err(|e| e.to_string())?);
    let req = Request::new_with_init(url, &init).map_err(|e| e.to_string())?;
    send(req, timeout_ms).await
}

#[derive(Deserialize)]
struct TokenResp {
    access_token: String,
    expires_in: i64,
}

/// Fetch a fresh OpenSky bearer token. `Ok(None)` when no secrets are set
/// (the caller then polls anonymously).
pub async fn fetch_opensky_token(env: &Env) -> std::result::Result<Option<(String, i64)>, String> {
    let (Ok(id), Ok(secret)) = (
        env.secret("OPENSKY_CLIENT_ID"),
        env.secret("OPENSKY_CLIENT_SECRET"),
    ) else {
        return Ok(None);
    };
    let body = format!(
        "grant_type=client_credentials&client_id={}&client_secret={}",
        form_encode(&id.to_string()),
        form_encode(&secret.to_string())
    );
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_headers(
            headers(&[("Content-Type", "application/x-www-form-urlencoded")])
                .map_err(|e| e.to_string())?,
        )
        .with_body(Some(JsValue::from_str(&body)));
    let req = Request::new_with_init(TOKEN_URL, &init).map_err(|e| e.to_string())?;
    let text = send(req, 10_000).await?;
    let t: TokenResp = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    Ok(Some((t.access_token, t.expires_in)))
}

/// Percent-encode for an `application/x-www-form-urlencoded` body.
fn form_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
