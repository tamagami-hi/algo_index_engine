mod access_token;
mod config;
mod dhan_api;
mod execution;
mod option_chain;
mod risk_engine;
mod server;

use anyhow::Result;

#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> Result<()> {
    config::load_env()?;
    server::install_tracing();

    tracing::info!(
        version = server::state::VERSION,
        profile = server::state::PROFILE,
        "algo_index_engine starting"
    );

    if std::env::args().nth(1).as_deref() == Some("--dhan-login") {
        dhan_api::dhan_oauth::login().await?;
        println!("Login complete. Start the engine without --dhan-login to run it.");
        return Ok(());
    }

    server::run().await
}
