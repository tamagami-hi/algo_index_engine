mod access_token;
mod config;
mod dhan_api;

use anyhow::Result;
use config::load_env;
use dhan_api::dhan_auth::get_dhan_credentials;
use dhan_api::dhan_ws::ws_dhan_connection;
use dhan_api::instrument_dl::download_instrument_master;
use dhan_api::instruments::{ChainUniverse, discovery_plan, ist_today, load_instrument_master};

#[tokio::main]
async fn main() -> Result<()> {
    load_env()?;

    let credentials = if std::env::args().nth(1).as_deref() == Some("--dhan-login") {
        dhan_api::dhan_oauth::login().await?
    } else {
        get_dhan_credentials().await?
    };

    let as_of = ist_today()?;
    let instrument_path = download_instrument_master(&as_of).await?;

    let master = load_instrument_master(&instrument_path)?;
    let universe = ChainUniverse::build(&master, &as_of)?;
    let discovery = discovery_plan(&universe);
    println!(
        "Instrument master: {} option contracts, {} spot rows from {} CSV rows",
        master.report.option_rows, master.report.spot_rows, master.report.total_rows
    );
    println!(
        "Option chains as of {as_of}: {} underlyings, {} with a spot instrument, {} unresolved",
        universe.chains.len(),
        universe.spots.len(),
        universe.unresolved.len()
    );
    println!(
        "Spot instruments to subscribe before the feed: {}",
        discovery.spot_subscriptions.len()
    );

    ws_dhan_connection(&credentials.client_id, &credentials.access_token).await?;

    Ok(())
}
