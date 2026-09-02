mod configs;
mod kite_api;

use anyhow::{Context, Result};
use configs::env_config::load_env;
use kite_api::kite_auth::get_token;
use kite_api::ws_protocol::kite_ws::ws_kite_connection;

#[tokio::main]
async fn main() -> Result<()> {
    load_env()?;

    let api_key =
        std::env::var("KITE_API_KEY").context("Missing KITE_API_KEY environment variable")?;
    let access_token = get_token().await?;

    println!("Received Kite access token: {}", &access_token);

    ws_kite_connection(&api_key, &access_token).await?;

    Ok(())
}
