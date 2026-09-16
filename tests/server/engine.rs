use super::reload_reason;
use crate::dhan_api::instruments::master::{
    ChainKind, InstrumentMaster, OptionContract, OptionType, SpotKind, SpotRow,
};
use crate::dhan_api::instruments::{ExchangeSegment, build_catalog, to_strike_units};

fn option(
    symbol: &str,
    expiry: &str,
    strike: f64,
    option_type: OptionType,
    id: &str,
) -> OptionContract {
    OptionContract {
        segment: ExchangeSegment::NseFno,
        security_id: id.to_owned(),
        underlying_symbol: symbol.to_owned(),
        expiry: expiry.to_owned(),
        strike_units: to_strike_units(strike),
        option_type,
        lot_size: 65,
        kind: ChainKind::Index,
    }
}

fn index_row(symbol: &str, id: &str) -> SpotRow {
    SpotRow {
        segment: ExchangeSegment::IdxI,
        security_id: id.to_owned(),
        exchange_id: "NSE".to_owned(),
        underlying_symbol: symbol.to_owned(),
        symbol_name: symbol.to_owned(),
        expiry: String::new(),
        kind: SpotKind::Index,
    }
}

fn catalog_for(expiry: &str, as_of: &str) -> crate::dhan_api::instruments::Catalog {
    let master = InstrumentMaster {
        options: vec![
            option("NIFTY", expiry, 25_000.0, OptionType::Call, "1"),
            option("NIFTY", expiry, 25_000.0, OptionType::Put, "2"),
        ],
        spots: vec![index_row("NIFTY", "13")],
        report: Default::default(),
    };
    build_catalog(&master, as_of).expect("catalog")
}

#[test]
fn nothing_loaded_means_load() {
    let reason = reload_reason(None, "2026-09-16").expect("must load");
    assert!(reason.contains("first load"), "{reason}");
    assert!(reason.contains("2026-09-16"), "{reason}");
}

#[test]
fn a_catalog_for_today_with_a_live_expiry_is_kept() {
    let loaded = (
        "2026-09-16".to_owned(),
        catalog_for("2026-09-22", "2026-09-16"),
    );
    assert_eq!(
        reload_reason(Some(&loaded), "2026-09-16"),
        None,
        "no reason to rebuild a current universe"
    );
}

#[test]
fn the_day_moving_forces_a_reload() {
    let loaded = (
        "2026-09-16".to_owned(),
        catalog_for("2026-09-22", "2026-09-16"),
    );
    let reason = reload_reason(Some(&loaded), "2026-09-17").expect("must reload");
    assert!(reason.contains("2026-09-16"), "{reason}");
    assert!(reason.contains("2026-09-17"), "{reason}");
}

#[test]
fn an_expiry_that_has_passed_forces_a_reload_so_the_next_one_is_picked_up() {
    let loaded = (
        "2026-09-22".to_owned(),
        catalog_for("2026-09-22", "2026-09-22"),
    );
    assert_eq!(
        reload_reason(Some(&loaded), "2026-09-22"),
        None,
        "expiry day is tradable, not stale"
    );

    let reason = reload_reason(Some(&loaded), "2026-09-23").expect("must reload");
    assert!(
        reason.contains("2026-09-22") && reason.contains("2026-09-23"),
        "the reason must name the dead expiry and the current day: {reason}"
    );
}

#[test]
fn a_stale_expiry_is_caught_even_when_the_day_label_still_matches() {
    let loaded = (
        "2026-09-23".to_owned(),
        catalog_for("2026-09-22", "2026-09-22"),
    );
    let reason = reload_reason(Some(&loaded), "2026-09-23").expect("must reload");
    assert!(
        reason.contains("NIFTY") && reason.contains("2026-09-22"),
        "{reason}"
    );
}

#[test]
fn stale_expiry_names_the_chain_at_fault() {
    let catalog = catalog_for("2026-09-22", "2026-09-22");
    assert_eq!(catalog.stale_expiry("2026-09-22"), None);
    assert_eq!(
        catalog.stale_expiry("2026-09-23"),
        Some(("NIFTY", "2026-09-22"))
    );
}
