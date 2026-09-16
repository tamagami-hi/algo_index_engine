use std::time::Duration;

use anyhow::Result;
use tokio_util::sync::CancellationToken;

use crate::dhan_api::dhan_auth::get_dhan_credentials;
use crate::dhan_api::dhan_ws::ws_dhan_connection;
use crate::dhan_api::instrument_dl::download_instrument_master;
use crate::dhan_api::instruments::{Catalog, build_catalog, ist_today, load_instrument_master};
use crate::option_chain::ChainBook;
use crate::server::report::report_catalog;
use crate::server::state::{EngineState, Phase};

const RETRY_MIN: Duration = Duration::from_secs(5);
const RETRY_MAX: Duration = Duration::from_secs(300);

pub(crate) async fn run(engine: EngineState, shutdown: CancellationToken) -> Result<()> {
    let mut backoff = RETRY_MIN;
    let mut loaded: Option<(String, Catalog)> = None;

    while !shutdown.is_cancelled() {
        match cycle(&engine, &mut loaded, &shutdown).await {
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
                eprintln!("engine cycle failed: {detail}");
                if engine.snapshot().phase != Phase::AuthFailed {
                    engine.feed_disconnected(detail);
                }
            }
        }

        println!("retrying in {}s", backoff.as_secs());
        tokio::select! {
            () = shutdown.cancelled() => break,
            () = tokio::time::sleep(backoff) => {}
        }
        backoff = (backoff * 2).min(RETRY_MAX);
    }

    println!("engine loop stopped");
    Ok(())
}

async fn cycle(
    engine: &EngineState,
    loaded: &mut Option<(String, Catalog)>,
    shutdown: &CancellationToken,
) -> Result<()> {
    engine.set_phase(Phase::Authenticating, "");
    let credentials = match get_dhan_credentials().await {
        Ok(credentials) => credentials,
        Err(error) => {
            let detail = format!("{error:#}");
            engine.set_phase(Phase::AuthFailed, detail.clone());
            anyhow::bail!(detail);
        }
    };

    let as_of = ist_today()?;
    let reason = reload_reason(loaded.as_ref(), &as_of);
    if let Some(reason) = reason {
        engine.set_phase(Phase::LoadingInstruments, reason.clone());
        println!("loading the universe: {reason}");
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
        shutdown,
    )
    .await
}

pub(crate) fn reload_reason(
    loaded: Option<&(String, Catalog)>,
    as_of: &str,
) -> Option<String> {
    match loaded {
        None => Some(format!("first load for trading day {as_of}")),
        Some((day, _)) if day != as_of => {
            Some(format!("trading day moved from {day} to {as_of}"))
        }
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
    println!(
        "Instrument master: {} option contracts, {} spot rows from {} CSV rows",
        master.report.option_rows, master.report.spot_rows, master.report.total_rows
    );
    let catalog = build_catalog(&master, as_of)?;
    let book = ChainBook::build(&master, &catalog);
    println!("Option chain book: {} chains assembled", book.chains());
    Ok((catalog, book))
}
