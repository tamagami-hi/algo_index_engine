pub(crate) mod engine;
pub(crate) mod http;
pub(crate) mod report;
pub(crate) mod state;

use std::time::Duration;

use anyhow::{Context, Result};
use tokio::signal::unix::{SignalKind, signal};
use tokio_util::sync::CancellationToken;

pub(crate) use state::EngineState;

const CHAIN_SUMMARY_INTERVAL: Duration = Duration::from_secs(1);

use state::Phase;

pub(crate) async fn run() -> Result<()> {
    let engine = EngineState::new();
    let addr = http::listen_addr()?;
    let shutdown = CancellationToken::new();

    let serving = tokio::spawn({
        let engine = engine.clone();
        let shutdown = shutdown.clone();
        async move {
            let served = http::serve(engine, addr, shutdown.clone()).await;
            shutdown.cancel();
            served
        }
    });

    let running = tokio::spawn({
        let engine = engine.clone();
        let shutdown = shutdown.clone();
        async move {
            let ran = engine::run(engine, shutdown.clone()).await;
            shutdown.cancel();
            ran
        }
    });

    let publishing = tokio::spawn({
        let engine = engine.clone();
        let shutdown = shutdown.clone();
        async move { publish_chain_summaries(engine, shutdown).await }
    });

    let mut terminate =
        signal(SignalKind::terminate()).context("cannot install the SIGTERM handler")?;

    tokio::select! {
        () = shutdown.cancelled() => {}
        _ = tokio::signal::ctrl_c() => begin_shutdown(&engine, &shutdown, "SIGINT"),
        _ = terminate.recv() => begin_shutdown(&engine, &shutdown, "SIGTERM"),
    }

    let served = serving.await.context("the HTTP server task panicked")?;
    let ran = running.await.context("the engine task panicked")?;
    publishing.await.context("the publisher task panicked")?;

    served.context("HTTP server stopped")?;
    ran.context("engine loop stopped")?;
    Ok(())
}

async fn publish_chain_summaries(engine: EngineState, shutdown: CancellationToken) {
    let mut ticker = tokio::time::interval(CHAIN_SUMMARY_INTERVAL);
    loop {
        tokio::select! {
            () = shutdown.cancelled() => return,
            _ = ticker.tick() => engine.publish_chain_summary(),
        }
    }
}

fn begin_shutdown(engine: &EngineState, shutdown: &CancellationToken, cause: &str) {
    println!("{cause} received, shutting down");
    engine.set_phase(Phase::ShuttingDown, format!("{cause} received"));
    shutdown.cancel();
}
