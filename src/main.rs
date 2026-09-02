mod configs;
mod kite_api;

use anyhow::Result;
use configs::env_config::load_env;
use kite_api::kite_auth::get_token;

#[tokio::main]
async fn main() -> Result<()> {
    load_env()?;

    let access_token = get_token().await?;

    println!("Received Kite access token: {}", &access_token);

    Ok(())
}
