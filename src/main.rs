mod access_token;
mod config;
mod dhan_api;

use anyhow::Result;
use config::load_env;
use dhan_api::dhan_auth::get_dhan_credentials;
use dhan_api::dhan_ws::ws_dhan_connection;
use dhan_api::instrument_dl::download_instrument_master;
use dhan_api::instruments::{
    Catalog, MAX_CONNECTIONS, MAX_PER_CONNECTION, build_catalog, ist_today, load_instrument_master,
};

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
    println!(
        "Instrument master: {} option contracts, {} spot rows from {} CSV rows",
        master.report.option_rows, master.report.spot_rows, master.report.total_rows
    );

    let catalog = build_catalog(&master, &as_of)?;
    report_catalog(&catalog);

    ws_dhan_connection(&credentials.client_id, &credentials.access_token).await?;

    Ok(())
}

fn report_catalog(catalog: &Catalog) {
    let report = &catalog.report;

    println!(
        "Front-expiry universe as of {}: {} indices, {} F&O stocks",
        report.as_of, report.index_underlyings, report.stock_underlyings
    );

    println!(
        "  spot           {:>6} instruments  {:>3} messages   ({} index, {} equity, {} index future)",
        catalog.spot.len(),
        catalog.spot.message_count(),
        report.spot_index,
        report.spot_equity,
        report.spot_index_future
    );
    println!(
        "  index options  {:>6} instruments  {:>3} messages   ({} chains, front expiry only)",
        catalog.index_options.len(),
        catalog.index_options.message_count(),
        report.index_chains.len()
    );

    for chain in &report.index_chains {
        println!(
            "        {:<9} {:<13} {}  {:>5} contracts",
            chain.underlying.segment.as_str(),
            chain.underlying.symbol,
            chain.expiry,
            chain.contracts
        );
    }

    for chain in &report.excluded_index_chains {
        println!(
            "        {:<9} {:<13} {}  {:>5} contracts  EXCLUDED",
            chain.underlying.segment.as_str(),
            chain.underlying.symbol,
            chain.expiry,
            chain.contracts
        );
    }

    let connections = catalog.connections_required();
    println!(
        "  total          {:>6} instruments  {:>3} messages   {} of {} connections",
        catalog.len(),
        catalog.message_count(),
        connections,
        MAX_CONNECTIONS
    );

    if connections == 1 {
        println!(
            "                 {} slots spare in the single connection",
            MAX_PER_CONNECTION - catalog.len()
        );
    } else {
        println!(
            "                 over one connection by {} instruments",
            catalog.len() - MAX_PER_CONNECTION
        );
    }

    if !report.unresolved.is_empty() {
        println!(
            "  {} underlying(s) with no resolvable spot instrument:",
            report.unresolved.len()
        );
        for key in &report.unresolved {
            println!("        {:<9} {}", key.segment.as_str(), key.symbol);
        }
    }
}
