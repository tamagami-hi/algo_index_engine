use std::convert::Infallible;
use std::net::SocketAddr;

use anyhow::{Context, Result};
use axum::{
    Router,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use futures_util::stream::{self, Stream};
use tokio_util::sync::CancellationToken;

use serde::{Deserialize, Serialize};

use crate::option_chain::book::ChainColumns;
use crate::risk_engine;
use crate::server::state::{EngineState, Snapshot};

#[derive(Debug, Deserialize)]
pub(crate) struct StreamParams {
    chain: Option<String>,
}

#[derive(Debug, Serialize)]
struct StreamFrame {
    #[serde(skip_serializing_if = "Option::is_none")]
    chain: Option<ChainColumns>,
    state: Snapshot,
}

const DEFAULT_ADDR: &str = "0.0.0.0:8081";
const ADDR_VARIABLE: &str = "BLACKBOX_HTTP_ADDR";

#[derive(Clone)]
struct Http {
    engine: EngineState,
    shutdown: CancellationToken,
}

pub(crate) fn listen_addr() -> Result<SocketAddr> {
    let raw = std::env::var(ADDR_VARIABLE).unwrap_or_else(|_| DEFAULT_ADDR.to_owned());
    raw.parse()
        .with_context(|| format!("{ADDR_VARIABLE} is not a valid socket address: {raw}"))
}

pub(crate) async fn serve(
    engine: EngineState,
    addr: SocketAddr,
    shutdown: CancellationToken,
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
            get(api_strategy).put(api_save_strategy).delete(api_delete_strategy),
        )
        .route("/api/strategies/{id}/resolve", get(api_resolve_strategy))
        .route("/api/strategies/{id}/activate", post(api_activate))
        .route("/api/strategies/{id}/deactivate", post(api_deactivate))
        .route("/api/active", get(api_active))
        .with_state(Http {
            engine,
            shutdown: shutdown.clone(),
        });

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("cannot bind the HTTP server to {addr}"))?;

    println!("HTTP server listening on {addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await
        .context("HTTP server failed")?;

    println!("HTTP server drained");
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

async fn health(State(http): State<Http>) -> Response {
    json(StatusCode::OK, encode(&http.engine))
}

async fn ready(State(http): State<Http>) -> Response {
    let snapshot = http.engine.snapshot();
    let status = if snapshot.phase.is_healthy() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    json(status, encode(&http.engine))
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
    let body = serde_json::to_string(&http.engine.chain_symbols()).unwrap_or_else(|_| "[]".to_owned());
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

async fn api_stream(
    State(http): State<Http>,
    Query(params): Query<StreamParams>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let receiver = http.engine.subscribe();
    let shutdown = http.shutdown;
    let engine = http.engine;
    let chain = params.chain;

    let stream = stream::unfold((receiver, true), move |(mut receiver, first)| {
        let shutdown = shutdown.clone();
        let engine = engine.clone();
        let chain = chain.clone();
        async move {
            if !first {
                tokio::select! {
                    () = shutdown.cancelled() => return None,
                    changed = receiver.changed() => changed.ok()?,
                }
            }
            let mut snapshot = receiver.borrow_and_update().clone();
            snapshot.uptime_seconds =
                (crate::server::state::now_millis() - snapshot.started_at_ms) / 1_000;

            let payload = StreamFrame {
                chain: chain.as_deref().and_then(|symbol| engine.chain_columns(symbol)),
                state: snapshot,
            };
            let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_owned());
            Some((
                Ok::<Event, Infallible>(Event::default().data(encoded)),
                (receiver, false),
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
        return encoded(StatusCode::UNPROCESSABLE_ENTITY, &problem);
    }
    match risk_engine::store::save(&strategy) {
        Ok(()) => encoded(StatusCode::OK, &strategy),
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
