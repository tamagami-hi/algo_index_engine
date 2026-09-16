use crate::dhan_api::instruments::{Catalog, MAX_INSTRUMENTS};

pub(crate) fn report_catalog(catalog: &Catalog) {
    let report = &catalog.report;

    println!(
        "Front-expiry index universe as of {}: {} indices",
        report.as_of, report.index_underlyings
    );

    println!(
        "  spot           {:>6} instruments  {:>3} messages   ({} index, {} index future)",
        catalog.spot.len(),
        catalog.spot.message_count(),
        report.spot_index,
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

    println!(
        "  total          {:>6} instruments  {:>3} messages   {} of {} on the single connection, {} spare",
        catalog.len(),
        catalog.message_count(),
        catalog.len(),
        MAX_INSTRUMENTS,
        catalog.spare_capacity()
    );

    if !report.unresolved.is_empty() {
        println!(
            "  {} underlying(s) with no resolvable spot instrument:",
            report.unresolved.len()
        );
        for key in &report.unresolved {
            println!("        {:<9} {}", key.segment.as_str(), key.symbol);
        }
    }

    if !report.missing_extra_spots.is_empty() {
        println!(
            "  {} extra spot index(es) missing from the instrument master:",
            report.missing_extra_spots.len()
        );
        for symbol in &report.missing_extra_spots {
            println!("        {symbol}");
        }
    }
}
