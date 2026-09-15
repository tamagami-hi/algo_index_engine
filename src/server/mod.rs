pub(crate) mod engine;
pub(crate) mod http;
pub(crate) mod report;
pub(crate) mod state;

use anyhow::{Context, Result};

pub(crate) use state::EngineState;

pub(crate) async fn run() -> Result<()> {
    let engine = EngineState::new();
    let addr = http::listen_addr()?;

    let serving = tokio::spawn({
        let engine = engine.clone();
        async move { http::serve(engine, addr).await }
    });

    let running = tokio::spawn({
        let engine = engine.clone();
        async move { engine::run(engine).await }
    });

    tokio::select! {
        joined = serving => {
            joined?.context("HTTP server stopped")?;
        }
        joined = running => {
            joined?.context("engine loop stopped")?;
        }
    }

    Ok(())
}
