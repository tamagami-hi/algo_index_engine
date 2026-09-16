mod access_token;
mod config;
mod dhan_api;
mod option_chain;
mod server;

use anyhow::Result;

#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> Result<()> {
    config::load_env()?;

    if std::env::args().nth(1).as_deref() == Some("--dhan-login") {
        dhan_api::dhan_oauth::login().await?;
        println!("Login complete. Start the engine without --dhan-login to run it.");
        return Ok(());
    }

    server::run().await
}
