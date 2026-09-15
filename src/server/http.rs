use std::convert::Infallible;
use std::net::SocketAddr;

use anyhow::{Context, Result};
use axum::{
    Router,
    extract::State,
    http::{StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::get,
};
use futures_util::stream::{self, Stream};

use crate::server::state::EngineState;

const DEFAULT_ADDR: &str = "0.0.0.0:8081";
const ADDR_VARIABLE: &str = "BLACKBOX_HTTP_ADDR";

pub(crate) fn listen_addr() -> Result<SocketAddr> {
    let raw = std::env::var(ADDR_VARIABLE).unwrap_or_else(|_| DEFAULT_ADDR.to_owned());
    raw.parse()
        .with_context(|| format!("{ADDR_VARIABLE} is not a valid socket address: {raw}"))
}

pub(crate) async fn serve(state: EngineState, addr: SocketAddr) -> Result<()> {
    let app = Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/api/state", get(api_state))
        .route("/api/stream", get(api_stream))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("cannot bind the HTTP server to {addr}"))?;

    println!("HTTP server listening on {addr}");

    axum::serve(listener, app)
        .await
        .context("HTTP server failed")
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

async fn health(State(state): State<EngineState>) -> Response {
    json(StatusCode::OK, encode(&state))
}

async fn ready(State(state): State<EngineState>) -> Response {
    let snapshot = state.snapshot();
    let status = if snapshot.phase.is_healthy() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    json(status, encode(&state))
}

async fn api_state(State(state): State<EngineState>) -> Response {
    json(StatusCode::OK, encode(&state))
}

async fn api_stream(State(state): State<EngineState>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let receiver = state.subscribe();

    let stream = stream::unfold((receiver, true), |(mut receiver, first)| async move {
        if !first && receiver.changed().await.is_err() {
            return None;
        }
        let mut snapshot = receiver.borrow_and_update().clone();
        snapshot.uptime_seconds = crate::server::state::now_unix() - snapshot.started_at;
        let payload = serde_json::to_string(&snapshot).unwrap_or_else(|_| "{}".to_owned());
        Some((
            Ok::<Event, Infallible>(Event::default().data(payload)),
            (receiver, false),
        ))
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}
