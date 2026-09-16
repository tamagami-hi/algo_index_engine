pub(crate) mod engine;
pub(crate) mod http;
pub(crate) mod report;
pub(crate) mod state;

use anyhow::{Context, Result};
use tokio::signal::unix::{SignalKind, signal};
use tokio_util::sync::CancellationToken;

pub(crate) use state::EngineState;

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


    let mut terminate =
        signal(SignalKind::terminate()).context("cannot install the SIGTERM handler")?;

    tokio::select! {
        () = shutdown.cancelled() => {}
        _ = tokio::signal::ctrl_c() => begin_shutdown(&engine, &shutdown, "SIGINT"),
        _ = terminate.recv() => begin_shutdown(&engine, &shutdown, "SIGTERM"),
    }

    let served = serving.await.context("the HTTP server task panicked")?;
    let ran = running.await.context("the engine task panicked")?;

    served.context("HTTP server stopped")?;
    ran.context("engine loop stopped")?;
    Ok(())
}


fn begin_shutdown(engine: &EngineState, shutdown: &CancellationToken, cause: &str) {
    println!("{cause} received, shutting down");
    engine.set_phase(Phase::ShuttingDown, format!("{cause} received"));
    shutdown.cancel();
}
