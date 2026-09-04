mod configs;
mod dhan_api;
mod server;
mod utils;

use anyhow::Result;
use configs::env_config::load_env;
use dhan_api::dhan_auth::get_dhan_credentials;
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

    ws_dhan_connection(&credentials.client_id, &credentials.access_token).await?;

    Ok(())
}
