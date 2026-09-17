use std::time::Duration;

use anyhow::Result;
use tokio_util::sync::CancellationToken;

use crate::dhan_api::dhan_auth::get_dhan_credentials;
use crate::dhan_api::dhan_oauth::shared_callback::SharedCallback;
use crate::dhan_api::dhan_ws::ws_dhan_connection;
use crate::dhan_api::instrument_dl::download_instrument_master;
use crate::dhan_api::instruments::{Catalog, build_catalog, ist_today, load_instrument_master};
use crate::option_chain::ChainBook;
use crate::server::report::report_catalog;
use crate::server::state::{EngineState, Phase};

const RETRY_MIN: Duration = Duration::from_secs(5);
const RETRY_MAX: Duration = Duration::from_secs(300);

pub(crate) async fn run(
    engine: EngineState,
    shutdown: CancellationToken,
    callback: Option<SharedCallback>,
) -> Result<()> {
    let mut backoff = RETRY_MIN;
    let mut loaded: Option<(String, Catalog)> = None;

    while !shutdown.is_cancelled() {
        match cycle(&engine, &mut loaded, &shutdown, callback.as_ref()).await {
            Ok(()) => {
                if shutdown.is_cancelled() {
                    break;
                }
                engine.feed_disconnected("the feed closed cleanly");
                backoff = RETRY_MIN;
            }
            Err(error) => {
                if shutdown.is_cancelled() {
                    break;
                }
                let detail = format!("{error:#}");
                tracing::error!(detail = %detail, "engine cycle failed");
                if engine.snapshot().phase != Phase::AuthFailed {
                    engine.feed_disconnected(detail);
                }
            }
        }

        // A login asked for from the interface must not sit behind a backoff that
        // has already doubled its way up to five minutes. The operator is waiting
        // at the screen for a consent link.
        if engine.login_requested() {
            backoff = RETRY_MIN;
            tracing::info!("a login was requested; retrying immediately");
            continue;
        }

        tracing::info!(seconds = backoff.as_secs(), "retrying");
        tokio::select! {
            () = shutdown.cancelled() => break,
            () = tokio::time::sleep(backoff) => {}
        }
        backoff = (backoff * 2).min(RETRY_MAX);
    }

    tracing::info!("engine loop stopped");
    Ok(())
}

async fn cycle(
    engine: &EngineState,
    loaded: &mut Option<(String, Catalog)>,
    shutdown: &CancellationToken,
    callback: Option<&SharedCallback>,
) -> Result<()> {
    // A child of the shutdown token, so a login request can end this cycle
    // without taking the HTTP server down with it. Cancelling the shared token
    // would stop the very interface the operator is using to log in.
    let cycle_token = shutdown.child_token();
    engine.register_cycle(cycle_token.clone());

    let forced = engine.take_login_request();
    if forced {
        if let Err(error) = crate::dhan_api::dhan_oauth::discard_session() {
            tracing::warn!(error = %format!("{error:#}"), "could not discard the saved session");
        } else {
            tracing::info!("saved Dhan session discarded for a requested fresh login");
        }
    }

    engine.set_phase(Phase::Authenticating, "");
    let announce = {
        let engine = engine.clone();
        tokio::spawn(async move {
            for _ in 0..600 {
                tokio::time::sleep(Duration::from_millis(500)).await;
                engine.login_awaiting_consent();
            }
        })
    };
    let authentication = tokio::select! {
        () = shutdown.cancelled() => { announce.abort(); return Ok(()); },
        result = get_dhan_credentials(callback) => result,
    };
    announce.abort();
    let credentials = match authentication {
        Ok(credentials) => credentials,
        Err(error) => {
            let detail = format!("{error:#}");
            engine.set_phase(Phase::AuthFailed, detail.clone());
            anyhow::bail!(detail);
        }
    };
    engine.login_completed();

    let as_of = ist_today()?;
    let reason = reload_reason(loaded.as_ref(), &as_of);
    if let Some(reason) = reason {
        engine.set_phase(Phase::LoadingInstruments, reason.clone());
        tracing::info!(reason = %reason, "loading the instrument universe");
        match load_universe(&as_of).await {
            Ok((catalog, book)) => {
                report_catalog(&catalog);
                engine.set_catalog(&as_of, &catalog);
                engine.set_book(book);
                *loaded = Some((as_of.clone(), catalog));
            }
            Err(error) => {
                let detail = format!("{error:#}");
                engine.set_phase(Phase::InstrumentsFailed, detail.clone());
                anyhow::bail!(detail);
            }
        }
    }

    let Some((_, catalog)) = loaded.as_ref() else {
        anyhow::bail!("no catalog is loaded for {as_of}");
    };

    engine.set_phase(Phase::Ready, "");
    ws_dhan_connection(
        &credentials.client_id,
        &credentials.access_token,
        catalog,
        &as_of,
        engine,
        &cycle_token,
    )
    .await
}

pub(crate) fn reload_reason(loaded: Option<&(String, Catalog)>, as_of: &str) -> Option<String> {
    match loaded {
        None => Some(format!("first load for trading day {as_of}")),
        Some((day, _)) if day != as_of => Some(format!("trading day moved from {day} to {as_of}")),
        Some((_, catalog)) => catalog
            .stale_expiry(as_of)
            .map(|(symbol, expiry)| format!("{symbol} expiry {expiry} is behind {as_of}")),
    }
}

#[cfg(test)]
#[path = "../../tests/server/engine.rs"]
mod tests;

async fn load_universe(as_of: &str) -> Result<(Catalog, ChainBook)> {
    let instrument_path = download_instrument_master(as_of).await?;
    let master = load_instrument_master(&instrument_path)?;
    tracing::info!(
        option_contracts = master.report.option_rows,
        spot_rows = master.report.spot_rows,
        csv_rows = master.report.total_rows,
        "instrument master loaded"
    );
    let catalog = build_catalog(&master, as_of)?;
    let book = ChainBook::build(&master, &catalog);
    tracing::info!(chains = book.chains(), "option chain book assembled");
    Ok((catalog, book))
}
