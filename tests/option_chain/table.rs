use super::*;
use crate::dhan_api::instruments::{
    ChainKind, ExchangeSegment, InstrumentMaster, OptionContract, OptionType, SpotInstrument,
    SpotKind, UnderlyingKey, to_strike_units,
};

fn contract(
    symbol: &str,
    expiry: &str,
    strike: f64,
    option_type: OptionType,
    security_id: &str,
) -> OptionContract {
    OptionContract {
        segment: ExchangeSegment::NseFno,
        security_id: security_id.to_owned(),
        underlying_symbol: symbol.to_owned(),
        expiry: expiry.to_owned(),
        strike_units: to_strike_units(strike),
        option_type,
        lot_size: 65,
        kind: ChainKind::Index,
    }
}

fn nifty_spot() -> SpotInstrument {
    SpotInstrument {
        segment: ExchangeSegment::IdxI,
        security_id: "13".to_owned(),
        kind: SpotKind::Index,
    }
}

#[test]
fn a_strike_quoted_on_only_one_side_still_gets_a_row() {
    let master = InstrumentMaster {
        options: vec![
            contract("NIFTY", "2026-09-22", 25_000.0, OptionType::Call, "1"),
            contract("NIFTY", "2026-09-22", 25_000.0, OptionType::Put, "2"),
            contract("NIFTY", "2026-09-22", 25_050.0, OptionType::Call, "3"),
            contract("NIFTY", "2026-09-22", 25_100.0, OptionType::Put, "4"),
        ],
        spots: Vec::new(),
        report: Default::default(),
    };

    let chains = vec![(
        UnderlyingKey::new(ExchangeSegment::NseFno, "NIFTY"),
        "2026-09-22".to_owned(),
        nifty_spot(),
    )];
    let tables = build_tables(&master, &chains);

    assert_eq!(tables.len(), 1);
    let table = &tables[0];
    assert_eq!(
        table.strikes,
        vec![25_000.0, 25_050.0, 25_100.0],
        "the union of call and put strikes, sorted"
    );
    assert_eq!(table.calls.security_id[1].as_deref(), Some("3"));
    assert_eq!(
        table.puts.security_id[1], None,
        "a missing side stays absent rather than aliasing another strike"
    );
    assert_eq!(table.calls.security_id[2], None);
    assert_eq!(table.puts.security_id[2].as_deref(), Some("4"));
    assert_eq!(table.lot_size, 65);
}

#[test]
fn only_the_requested_expiry_and_underlying_enter_the_table() {
    let master = InstrumentMaster {
        options: vec![
            contract("NIFTY", "2026-09-22", 25_000.0, OptionType::Call, "keep"),
            contract("NIFTY", "2026-09-29", 25_000.0, OptionType::Call, "next-expiry"),
            contract("BANKNIFTY", "2026-09-22", 25_000.0, OptionType::Call, "other"),
        ],
        spots: Vec::new(),
        report: Default::default(),
    };

    let chains = vec![(
        UnderlyingKey::new(ExchangeSegment::NseFno, "NIFTY"),
        "2026-09-22".to_owned(),
        nifty_spot(),
    )];
    let tables = build_tables(&master, &chains);

    assert_eq!(tables[0].len(), 1);
    assert_eq!(tables[0].calls.security_id[0].as_deref(), Some("keep"));
}

#[test]
fn strike_step_is_the_median_gap_so_one_missing_strike_does_not_skew_it() {
    let units: Vec<i64> = [25_000.0, 25_050.0, 25_100.0, 25_200.0, 25_250.0]
        .iter()
        .copied()
        .map(to_strike_units)
        .collect();
    assert_eq!(strike_step_units(&units), to_strike_units(50.0));

    assert_eq!(strike_step_units(&[]), 0);
    assert_eq!(strike_step_units(&[to_strike_units(25_000.0)]), 0);
}

#[test]
fn nearest_strike_lookup_clamps_and_prefers_the_lower_strike_on_a_tie() {
    let master = InstrumentMaster {
        options: vec![
            contract("NIFTY", "2026-09-22", 25_000.0, OptionType::Call, "a"),
            contract("NIFTY", "2026-09-22", 25_050.0, OptionType::Call, "b"),
            contract("NIFTY", "2026-09-22", 25_100.0, OptionType::Call, "c"),
        ],
        spots: Vec::new(),
        report: Default::default(),
    };
    let chains = vec![(
        UnderlyingKey::new(ExchangeSegment::NseFno, "NIFTY"),
        "2026-09-22".to_owned(),
        nifty_spot(),
    )];
    let table = &build_tables(&master, &chains)[0];

    assert_eq!(
        table.find_nearest_strike_index(to_strike_units(25_025.0)),
        Some(0),
        "an exact midpoint resolves to the lower strike"
    );
    assert_eq!(
        table.find_nearest_strike_index(to_strike_units(25_026.0)),
        Some(1)
    );
    assert_eq!(
        table.find_nearest_strike_index(to_strike_units(1.0)),
        Some(0),
        "below the chain clamps to the first strike"
    );
    assert_eq!(
        table.find_nearest_strike_index(to_strike_units(99_999.0)),
        Some(2),
        "above the chain clamps to the last strike"
    );
}
