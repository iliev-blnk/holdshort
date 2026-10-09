//! Cloudflare Worker entry: routes `/health` to the ingest Durable Object and
//! pokes it from a cron trigger so the alarm loop can never stay dead.

use worker::*;

mod http;
mod ingest_do;

pub use ingest_do::Ingest;

fn ingest_stub(env: &Env) -> Result<Stub> {
    env.durable_object("INGEST")?
        .id_from_name("ltfm")?
        .get_stub()
}

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    if req.method() == Method::Get && req.path() == "/health" {
        return ingest_stub(&env)?
            .fetch_with_str("https://ingest/health")
            .await;
    }
    Response::error("Not found", 404)
}

#[event(scheduled)]
async fn scheduled(_event: ScheduledEvent, env: Env, _ctx: ScheduleContext) {
    let result = async {
        let req = Request::new("https://ingest/ensure", Method::Post)?;
        ingest_stub(&env)?.fetch_with_request(req).await
    }
    .await;
    if let Err(e) = result {
        console_error!("ensure failed: {e}");
    }
}
