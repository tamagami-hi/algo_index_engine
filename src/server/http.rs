use std::convert::Infallible;
use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::{
    Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use futures_util::stream::{self, Stream};
use tokio::sync::watch;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use serde::{Deserialize, Serialize};

use tower_http::services::{ServeDir, ServeFile};

use crate::dhan_api::dhan_oauth::shared_callback::SharedCallback;
use crate::execution::postback;
use crate::option_chain::book::ChainColumns;
use crate::risk_engine;
use crate::server::state::{EngineState, PostbackOutcome, Snapshot};

#[derive(Debug, Deserialize)]
pub(crate) struct StreamParams {
    chain: Option<String>,
}

#[derive(Debug, Serialize)]
struct StreamFrame {
    sequence: u64,
    published_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    feed_age_ms: Option<u64>,
    feed_silence_limit_ms: u64,
    publish_interval_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    chain: Option<ChainColumns>,
    state: Snapshot,
}

#[cfg(test)]
#[path = "../../tests/server/shared_callback.rs"]
mod shared_callback_tests;

#[cfg(test)]
#[path = "../../tests/server/postback.rs"]
mod postback_tests;
const WEB_ROOT: &str = "web/dist";
const ADDR_VARIABLE: &str = "BLACKBOX_HTTP_ADDR";
const PUBLISH_INTERVAL_VARIABLE: &str = "BLACKBOX_PUBLISH_INTERVAL_MS";
const DEFAULT_PUBLISH_INTERVAL_MS: u64 = 50;

fn publish_interval() -> Duration {
    let millis = std::env::var(PUBLISH_INTERVAL_VARIABLE)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_PUBLISH_INTERVAL_MS);
    Duration::from_millis(millis)
}

struct StreamGuard {
    chain: Option<String>,
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        tracing::info!(chain = ?self.chain, "stream client disconnected");
    }
}

struct StreamState {
    receiver: watch::Receiver<Snapshot>,
    first: bool,
    last_publish: Option<Instant>,
    _guard: StreamGuard,
}

#[derive(Clone)]
struct Http {
    engine: EngineState,
    shutdown: CancellationToken,
}

pub(crate) fn listen_addr() -> Result<SocketAddr> {
    let raw = std::env::var(ADDR_VARIABLE).with_context(|| {
        format!("Missing {ADDR_VARIABLE}; configure the backend address in .env")
    })?;
    let addr: SocketAddr = raw
        .parse()
        .with_context(|| format!("{ADDR_VARIABLE} is not a valid socket address"))?;
    anyhow::ensure!(addr.port() != 0, "{ADDR_VARIABLE} requires a nonzero port");
    Ok(addr)
}

pub(crate) async fn serve(
    engine: EngineState,
    listener: tokio::net::TcpListener,
    shutdown: CancellationToken,
    callback: Option<SharedCallback>,
) -> Result<()> {
    let app = Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/api/state", get(api_state))
        .route("/api/stream", get(api_stream))
        .route("/api/chains", get(api_chains))
        .route("/api/chain/{symbol}", get(api_chain))
        .route("/api/chain/{symbol}/columns", get(api_chain_columns))
        .route("/api/symbols", get(api_symbols))
        .route("/api/strategies", get(api_strategies))
        .route("/api/strategies/template", get(api_strategy_template))
        .route(
            "/api/strategies/{id}",
            get(api_strategy)
                .put(api_save_strategy)
                .delete(api_delete_strategy),
        )
        .route("/api/strategies/{id}/resolve", get(api_resolve_strategy))
        .route("/api/strategies/{id}/activate", post(api_activate))
        .route("/api/strategies/{id}/deactivate", post(api_deactivate))
        .route("/api/active", get(api_active))
        .route(
            postback::POSTBACK_PATH,
            post(dhan_postback).layer(DefaultBodyLimit::max(postback::POSTBACK_BODY_LIMIT_BYTES)),
        )
        .with_state(Http {
            engine,
            shutdown: shutdown.clone(),
        });
    let app = match callback {
        Some(callback) => app.merge(callback.router()),
        None => app,
    };

    let web_root = crate::config::data_path(WEB_ROOT);
    let app = if web_root.is_dir() {
        tracing::info!(root = %web_root.display(), "serving the frontend");
        let index = web_root.join("index.html");
        app.fallback_service(ServeDir::new(&web_root).fallback(ServeFile::new(index)))
    } else {
        tracing::warn!(
            root = %web_root.display(),
            "no frontend build present; serving the API only (cd web && npm run build)"
        );
        app
    };

    let addr = listener
        .local_addr()
        .context("cannot read the HTTP listener address")?;

    tracing::info!(%addr, "HTTP server listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await
        .context("HTTP server failed")?;

    tracing::info!("HTTP server drained");
    Ok(())
}

fn json(status: StatusCode, body: String) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response()
}

fn encode(state: &EngineState) -> String {
    serde_json::to_string(&state.snapshot()).unwrap_or_else(|_| "{}".to_owned())
}

#[derive(Debug, Serialize)]
struct Liveness {
    alive: bool,
    version: &'static str,
    profile: &'static str,
    phase: &'static str,
    uptime_seconds: i64,
}

#[derive(Debug, Serialize)]
struct Readiness {
    ready: bool,
    phase: &'static str,
    detail: String,
    reasons: Vec<&'static str>,
    catalog_loaded: bool,
    chains: usize,
    feed_connected: bool,
    last_frame_age_ms: Option<u64>,
}

async fn health(State(http): State<Http>) -> Response {
    let snapshot = http.engine.snapshot();
    encoded(
        StatusCode::OK,
        &Liveness {
            alive: true,
            version: snapshot.version,
            profile: crate::server::state::PROFILE,
            phase: snapshot.phase_label,
            uptime_seconds: snapshot.uptime_seconds,
        },
    )
}

async fn ready(State(http): State<Http>) -> Response {
    let snapshot = http.engine.snapshot();
    let mut reasons = Vec::new();

    if snapshot.phase == crate::server::state::Phase::Failed {
        reasons.push("a critical task has stopped");
    }
    if !snapshot.phase.is_healthy() {
        reasons.push("the engine is not in a ready phase");
    }
    if snapshot.catalog.is_none() {
        reasons.push("no instrument universe is loaded");
    }
    if snapshot.feed.chains == 0 {
        reasons.push("no option chains are assembled");
    }
    if !snapshot.feed.connected {
        reasons.push("the market feed is not connected");
    }
    if snapshot.feed.stale {
        reasons.push("the market feed has gone silent");
    }

    let last_frame_age_ms = snapshot
        .feed
        .last_frame_at
        .map(crate::option_chain::quality::age_since);
    let silence_limit = crate::option_chain::quality::freshness().feed_silence_max_ms;
    match last_frame_age_ms {
        None if snapshot.feed.connected => reasons.push("no market data has arrived yet"),
        Some(age) if age > silence_limit && !snapshot.feed.stale => {
            reasons.push("the market feed has gone silent");
        }
        _ => {}
    }

    let ready = reasons.is_empty();
    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    encoded(
        status,
        &Readiness {
            ready,
            phase: snapshot.phase_label,
            detail: snapshot.detail.clone(),
            reasons,
            catalog_loaded: snapshot.catalog.is_some(),
            chains: snapshot.feed.chains,
            feed_connected: snapshot.feed.connected,
            last_frame_age_ms,
        },
    )
}

async fn api_state(State(http): State<Http>) -> Response {
    json(StatusCode::OK, encode(&http.engine))
}

async fn api_chains(State(http): State<Http>) -> Response {
    let metrics = http.engine.chain_metrics();
    let body = serde_json::to_string(&metrics).unwrap_or_else(|_| "[]".to_owned());
    json(StatusCode::OK, body)
}

async fn api_symbols(State(http): State<Http>) -> Response {
    let body =
        serde_json::to_string(&http.engine.chain_symbols()).unwrap_or_else(|_| "[]".to_owned());
    json(StatusCode::OK, body)
}

async fn api_chain_columns(State(http): State<Http>, Path(symbol): Path<String>) -> Response {
    match http.engine.chain_columns(&symbol) {
        Some(columns) => {
            let body = serde_json::to_string(&columns).unwrap_or_else(|_| "{}".to_owned());
            json(StatusCode::OK, body)
        }
        None => json(
            StatusCode::NOT_FOUND,
            format!("{{\"error\":\"no chain for {symbol}\"}}"),
        ),
    }
}

async fn api_chain(State(http): State<Http>, Path(symbol): Path<String>) -> Response {
    match http.engine.chain_view(&symbol) {
        Some(view) => {
            let body = serde_json::to_string(&view).unwrap_or_else(|_| "{}".to_owned());
            json(StatusCode::OK, body)
        }
        None => json(
            StatusCode::NOT_FOUND,
            format!("{{\"error\":\"no chain for {symbol}\"}}"),
        ),
    }
}

async fn wait_to_publish(
    receiver: &mut watch::Receiver<Snapshot>,
    last_publish: Option<Instant>,
    interval: Duration,
    shutdown: &CancellationToken,
) -> bool {
    tokio::select! {
        () = shutdown.cancelled() => return false,
        changed = receiver.changed() => {
            if changed.is_err() {
                return false;
            }
        }
    }

    if let Some(previous) = last_publish {
        let earliest = previous + interval;
        if Instant::now() < earliest {
            tokio::select! {
                () = shutdown.cancelled() => return false,
                () = tokio::time::sleep_until(earliest) => {}
            }
        }
    }

    true
}

async fn api_stream(
    State(http): State<Http>,
    Query(params): Query<StreamParams>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let shutdown = http.shutdown;
    let engine = http.engine;
    let chain = params.chain;
    let interval = publish_interval();

    tracing::info!(chain = ?chain, interval_ms = interval.as_millis(), "stream client connected");

    let start = StreamState {
        receiver: engine.subscribe(),
        first: true,
        last_publish: None,
        _guard: StreamGuard {
            chain: chain.clone(),
        },
    };

    let stream = stream::unfold(start, move |mut state| {
        let shutdown = shutdown.clone();
        let engine = engine.clone();
        let chain = chain.clone();
        async move {
            if state.first {
                state.first = false;
            } else if !wait_to_publish(&mut state.receiver, state.last_publish, interval, &shutdown)
                .await
            {
                return None;
            }

            state.last_publish = Some(Instant::now());

            let mut snapshot = state.receiver.borrow_and_update().clone();
            snapshot.uptime_seconds =
                (crate::server::state::now_millis() - snapshot.started_at_ms) / 1_000;

            let feed_age_ms = snapshot
                .feed
                .last_frame_at
                .map(crate::option_chain::quality::age_since);

            let payload = StreamFrame {
                sequence: snapshot.sequence,
                published_at_ms: crate::server::state::now_millis(),
                feed_age_ms,
                feed_silence_limit_ms: crate::option_chain::quality::freshness()
                    .feed_silence_max_ms,
                publish_interval_ms: interval.as_millis() as u64,
                chain: chain
                    .as_deref()
                    .and_then(|symbol| engine.chain_columns(symbol)),
                state: snapshot,
            };
            let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_owned());
            Some((
                Ok::<Event, Infallible>(Event::default().data(encoded)),
                state,
            ))
        }
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn encoded(status: StatusCode, value: &impl Serialize) -> Response {
    match serde_json::to_string(value) {
        Ok(body) => json(status, body),
        Err(error) => json(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{{\"error\":\"cannot encode response: {error}\"}}"),
        ),
    }
}

fn failed(status: StatusCode, error: impl std::fmt::Display) -> Response {
    let message = error.to_string().replace('"', "'").replace('\n', " ");
    json(status, format!("{{\"error\":\"{message}\"}}"))
}

#[cfg(test)]
#[path = "../../tests/server/http.rs"]
mod tests;

async fn api_strategies() -> Response {
    encoded(StatusCode::OK, &risk_engine::store::list())
}

async fn api_strategy_template(Query(params): Query<StreamParams>) -> Response {
    let underlying = params.chain.unwrap_or_else(|| "NIFTY".to_owned());
    encoded(
        StatusCode::OK,
        &risk_engine::strategy::Strategy::template(&underlying),
    )
}

async fn api_active() -> Response {
    encoded(StatusCode::OK, &risk_engine::store::active())
}

async fn api_strategy(Path(id): Path<String>) -> Response {
    match risk_engine::store::load(&id) {
        Ok(strategy) => encoded(StatusCode::OK, &strategy),
        Err(error) => failed(StatusCode::NOT_FOUND, error),
    }
}

async fn api_save_strategy(Path(id): Path<String>, body: String) -> Response {
    let mut strategy: risk_engine::strategy::Strategy = match serde_json::from_str(&body) {
        Ok(strategy) => strategy,
        Err(error) => return failed(StatusCode::BAD_REQUEST, error),
    };
    strategy.id = id;

    if let Err(problem) = strategy.validate() {
        tracing::warn!(
            strategy = %strategy.id,
            problem = %serde_json::to_string(&problem).unwrap_or_default(),
            "rejected an invalid strategy"
        );
        return encoded(StatusCode::UNPROCESSABLE_ENTITY, &problem);
    }
    match risk_engine::store::save(&strategy) {
        Ok(()) => {
            tracing::info!(strategy = %strategy.id, underlying = %strategy.underlying, "strategy saved");
            encoded(StatusCode::OK, &strategy)
        }
        Err(error) => failed(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

async fn api_delete_strategy(Path(id): Path<String>) -> Response {
    match risk_engine::store::remove(&id) {
        Ok(()) => json(StatusCode::OK, "{\"removed\":true}".to_owned()),
        Err(error) => failed(StatusCode::NOT_FOUND, error),
    }
}

async fn api_activate(Path(id): Path<String>) -> Response {
    match risk_engine::store::activate(&id) {
        Ok(ids) => encoded(StatusCode::OK, &ids),
        Err(error) => failed(StatusCode::NOT_FOUND, error),
    }
}

async fn api_deactivate(Path(id): Path<String>) -> Response {
    match risk_engine::store::deactivate(&id) {
        Ok(ids) => encoded(StatusCode::OK, &ids),
        Err(error) => failed(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

async fn api_resolve_strategy(State(http): State<Http>, Path(id): Path<String>) -> Response {
    let strategy = match risk_engine::store::load(&id) {
        Ok(strategy) => strategy,
        Err(error) => return failed(StatusCode::NOT_FOUND, error),
    };
    match http.engine.resolve_strategy(&strategy) {
        Some(resolution) => encoded(StatusCode::OK, &resolution),
        None => failed(
            StatusCode::SERVICE_UNAVAILABLE,
            format!("no live chain for {}", strategy.underlying),
        ),
    }
}

const SOURCE_HEADER: &str = "x-forwarded-for";
const SOURCE_MAX_CHARS: usize = 120;

fn postback_source(headers: &header::HeaderMap) -> Option<String> {
    let raw = headers
        .get(SOURCE_HEADER)
        .and_then(|value| value.to_str().ok())?
        .trim();
    if raw.is_empty() {
        return None;
    }
    Some(raw.chars().take(SOURCE_MAX_CHARS).collect())
}

async fn dhan_postback(
    State(http): State<Http>,
    headers: header::HeaderMap,
    body: String,
) -> Response {
    let source = postback_source(&headers);
    let bytes = body.len();

    if bytes > postback::MAX_POSTBACK_BYTES {
        http.engine
            .postback_received(PostbackOutcome::Oversized, bytes, None);
        tracing::warn!(
            bytes,
            limit = postback::MAX_POSTBACK_BYTES,
            source = ?source,
            "rejected an oversized order postback"
        );
        return failed(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "postback body is {bytes} bytes; the limit is {}",
                postback::MAX_POSTBACK_BYTES
            ),
        );
    }

    match postback::record(&body, source.as_deref()) {
        Ok(stored) => {
            http.engine
                .postback_received(PostbackOutcome::Stored, bytes, Some(&stored));
            tracing::info!(
                bytes,
                day = %stored.day,
                records = stored.records,
                source = ?source,
                "stored an order postback"
            );
            encoded(
                StatusCode::OK,
                &serde_json::json!({ "stored": true, "records": stored.records }),
            )
        }
        Err(error) => {
            http.engine
                .postback_received(PostbackOutcome::Unwritable, bytes, None);
            tracing::error!(
                bytes,
                source = ?source,
                error = %format!("{error:#}"),
                "cannot store an order postback"
            );
            failed(
                StatusCode::INTERNAL_SERVER_ERROR,
                "cannot store the postback",
            )
        }
    }
}
