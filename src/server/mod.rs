pub(crate) mod engine;
pub(crate) mod http;
pub(crate) mod report;
pub(crate) mod state;
pub(crate) mod supervisor;

use anyhow::{Context, Result, bail};
use tokio::signal::unix::{SignalKind, signal};
use tokio_util::sync::CancellationToken;

pub(crate) use state::EngineState;

use state::Phase;
use supervisor::{Stop, Supervisor, wait_for_stop};

const DRAIN_GRACE: std::time::Duration = std::time::Duration::from_secs(10);

pub(crate) async fn run() -> Result<()> {
    let engine = EngineState::new();
    let addr = http::listen_addr()?;
    let callback = crate::dhan_api::dhan_auth::configured_callback(addr)?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("cannot bind the HTTP server to {addr}"))?;
    let shutdown = CancellationToken::new();

    if let Err(error) = crate::risk_engine::store::migrate() {
        tracing::warn!(error = %format!("{error:#}"), "could not migrate the strategy store");
    }

    let mut supervisor = Supervisor::new();

    supervisor.spawn("http server", {
        let engine = engine.clone();
        let shutdown = shutdown.clone();
        let callback = callback.clone();
        async move { http::serve(engine, listener, shutdown, callback).await }
    });

    supervisor.spawn("engine loop", {
        let engine = engine.clone();
        let shutdown = shutdown.clone();
        async move { engine::run(engine, shutdown, callback).await }
    });

    let stop = wait_for_stop(&mut supervisor, unix_signal()).await;

    let fault = match &stop {
        Stop::Signal(cause) => {
            tracing::info!(cause = %cause, "shutting down");
            engine.set_phase(Phase::ShuttingDown, format!("{cause} received"));
            None
        }
        Stop::Task { name, ended } if ended.is_fault() => {
            tracing::error!(
                task = %name,
                detail = %ended.detail(),
                "a critical task ended; taking the service down"
            );
            engine.set_phase(Phase::Failed, format!("{name} {}", ended.detail()));
            Some(format!("{name} {}", ended.detail()))
        }
        Stop::Task { name, ended } => {
            tracing::error!(
                task = %name,
                detail = %ended.detail(),
                "a critical task exited on its own; taking the service down"
            );
            engine.set_phase(Phase::Failed, format!("{name} {}", ended.detail()));
            Some(format!("{name} {}", ended.detail()))
        }
        Stop::Drained => {
            engine.set_phase(Phase::Failed, "every task exited".to_owned());
            Some("every critical task exited".to_owned())
        }
    };

    shutdown.cancel();

    for (name, ended) in supervisor.drain_within(DRAIN_GRACE).await {
        if ended.is_fault() {
            tracing::error!(task = %name, detail = %ended.detail(), "task ended badly during shutdown");
        } else {
            tracing::info!(task = %name, "task stopped");
        }
    }

    match fault {
        Some(detail) => bail!("{detail}"),
        None => Ok(()),
    }
}

async fn unix_signal() -> String {
    let mut terminate = match signal(SignalKind::terminate()) {
        Ok(stream) => stream,
        Err(error) => {
            tracing::error!(error = %error, "cannot install the SIGTERM handler");
            return std::future::pending().await;
        }
    };

    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            if let Err(error) = result {
                tracing::error!(error = %error, "cannot listen for SIGINT");
                return std::future::pending().await;
            }
            "SIGINT".to_owned()
        }
        _ = terminate.recv() => "SIGTERM".to_owned(),
    }
}

pub(crate) fn install_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};

    let filter = EnvFilter::try_from_env("BLACKBOX_LOG")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let installed = fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_ansi(false)
        .try_init();

    if let Err(error) = installed {
        eprintln!("could not install the log subscriber: {error}");
    }
}
