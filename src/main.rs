mod configs;
mod dhan_api;
mod server;
mod utils;

use anyhow::Result;
use configs::env_config::load_env;
use dhan_api::dhan_auth::get_dhan_credentials;
use dhan_api::instruments::{
    ChainUniverse, discovery_plan, ist_today, load_instrument_master,
};
use dhan_api::rest_protocol::instrument_dl::download_instrument_master;
use dhan_api::ws_protocol::dhan_ws::ws_dhan_connection;

#[tokio::main]
async fn main() -> Result<()> {
    load_env()?;

    let credentials = get_dhan_credentials().await?;

    let instrument_path = download_instrument_master().await?;
    println!(
        "Dhan instrument master saved to {}",
        instrument_path.display()
    );

    // TODO: get the full instrument master list to subscribe to the websocket.
    let master = load_instrument_master(&instrument_path)?;
    let as_of = ist_today()?;
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
