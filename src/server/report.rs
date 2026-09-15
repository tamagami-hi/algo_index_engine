use crate::dhan_api::instruments::{Catalog, MAX_CONNECTIONS, MAX_PER_CONNECTION};

pub(crate) fn report_catalog(catalog: &Catalog) {
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
