use super::*;
use crate::dhan_api::instruments::master::{OptionType, SpotRow};

const AS_OF: &str = "2026-09-15";

fn option(
    segment: ExchangeSegment,
    symbol: &str,
    expiry: &str,
    strike: i64,
    option_type: OptionType,
    kind: ChainKind,
    security_id: &str,
) -> OptionContract {
    OptionContract {
        segment,
        security_id: security_id.into(),
        underlying_symbol: symbol.into(),
        expiry: expiry.into(),
        strike_units: strike,
        option_type,
        lot_size: 50,
        kind,
    }
}

fn spot(
    segment: ExchangeSegment,
    exchange_id: &str,
    symbol: &str,
    security_id: &str,
    kind: SpotKind,
) -> SpotRow {
    SpotRow {
        segment,
        security_id: security_id.into(),
        exchange_id: exchange_id.into(),
        underlying_symbol: symbol.into(),
        symbol_name: symbol.into(),
        expiry: String::new(),
        kind,
    }
}

fn master(options: Vec<OptionContract>, spots: Vec<SpotRow>) -> InstrumentMaster {
    InstrumentMaster {
        options,
        spots,
        report: Default::default(),
    }
}

fn nifty_spot() -> SpotRow {
    spot(ExchangeSegment::IdxI, "NSE", "NIFTY", "13", SpotKind::Index)
}

fn ids(pool: &Pool) -> Vec<&str> {
    pool.instruments
        .iter()
        .map(|s| s.security_id.as_str())
        .collect()
}

#[test]
fn messages_never_exceed_the_hundred_instrument_limit_and_lose_nothing() {
    let mut options = Vec::new();
    for index in 0..250u32 {
        options.push(option(
            ExchangeSegment::NseFno,
            "NIFTY",
            "2026-09-29",
            i64::from(index) * 5_000_000,
            if index % 2 == 0 {
                OptionType::Call
            } else {
                OptionType::Put
            },
            ChainKind::Index,
            &format!("{}", 100_000 + index),
        ));
    }

    let catalog = build_catalog(&master(options, vec![nifty_spot()]), AS_OF).unwrap();

    assert_eq!(catalog.index_options.len(), 250);
    assert_eq!(catalog.index_options.message_count(), 3);

    let sizes: Vec<usize> = catalog.index_options.messages().map(<[_]>::len).collect();
    assert_eq!(sizes, vec![100, 100, 50]);
    for message in catalog.index_options.messages() {
        assert!(message.len() <= MAX_PER_MESSAGE);
    }

    let flattened: Vec<&Subscription> = catalog.index_options.messages().flatten().collect();
    assert_eq!(flattened.len(), catalog.index_options.instruments.len());
    assert!(
        flattened
            .iter()
            .zip(&catalog.index_options.instruments)
            .all(|(left, right)| *left == right)
    );
}

#[test]
fn index_pool_excludes_stock_options_and_expired_contracts() {
    let options = vec![
        option(
            ExchangeSegment::NseFno,
            "NIFTY",
            AS_OF,
            2_400_000_000,
            OptionType::Call,
            ChainKind::Index,
            "35070",
        ),
        option(
            ExchangeSegment::NseFno,
            "NIFTY",
            "2026-09-01",
            2_400_000_000,
            OptionType::Call,
            ChainKind::Index,
            "34000",
        ),
        option(
            ExchangeSegment::NseFno,
            "RELIANCE",
            "2026-09-29",
            140_000_000,
            OptionType::Call,
            ChainKind::Stock,
            "49081",
        ),
    ];
    let spots = vec![
        nifty_spot(),
        spot(
            ExchangeSegment::NseEq,
            "NSE",
            "RELIANCE",
            "2885",
            SpotKind::Equity,
        ),
    ];

    let catalog = build_catalog(&master(options, spots), AS_OF).unwrap();

    assert_eq!(
        ids(&catalog.index_options),
        vec!["35070"],
        "expiry equal to as_of stays, a past expiry goes"
    );
    assert!(
        !ids(&catalog.index_options).contains(&"49081"),
        "a stock option leaked into the index pool"
    );

    let spot_ids = ids(&catalog.spot);
    assert_eq!(spot_ids, vec!["13"], "only index spot is subscribed");
    assert!(
        !spot_ids.contains(&"2885"),
        "an F&O stock spot must never be subscribed"
    );
    assert_eq!(catalog.report.index_underlyings, 1);
    assert_eq!(catalog.report.spot_index, 1);
}

#[test]
fn only_the_front_expiry_is_subscribed_per_underlying() {
    let options = vec![
        option(
            ExchangeSegment::NseFno,
            "NIFTY",
            "2026-09-29",
            2_400_000_000,
            OptionType::Call,
            ChainKind::Index,
            "nifty-front",
        ),
        option(
            ExchangeSegment::NseFno,
            "NIFTY",
            "2026-10-27",
            2_400_000_000,
            OptionType::Call,
            ChainKind::Index,
            "nifty-back",
        ),
        option(
            ExchangeSegment::NseFno,
            "NIFTY",
            "2026-11-24",
            2_400_000_000,
            OptionType::Call,
            ChainKind::Index,
            "nifty-far",
        ),
        option(
            ExchangeSegment::BseFno,
            "SENSEX",
            "2026-09-17",
            8_460_000_000,
            OptionType::Put,
            ChainKind::Index,
            "sensex-front",
        ),
        option(
            ExchangeSegment::BseFno,
            "SENSEX",
            "2026-09-24",
            8_460_000_000,
            OptionType::Put,
            ChainKind::Index,
            "sensex-back",
        ),
    ];
    let spots = vec![
        nifty_spot(),
        spot(ExchangeSegment::IdxI, "BSE", "SENSEX", "51", SpotKind::Index),
    ];

    let catalog = build_catalog(&master(options, spots), AS_OF).unwrap();
    let selected = ids(&catalog.index_options);

    assert_eq!(selected.len(), 2, "one front expiry per underlying");
    assert!(selected.contains(&"nifty-front"));
    assert!(selected.contains(&"sensex-front"));
    assert!(!selected.contains(&"nifty-back"));
    assert!(!selected.contains(&"nifty-far"));
    assert!(!selected.contains(&"sensex-back"));

    let chains = &catalog.report.index_chains;
    assert_eq!(chains.len(), 2);
    let listed: Vec<(&str, &str)> = chains
        .iter()
        .map(|c| (c.underlying.symbol.as_str(), c.expiry.as_str()))
        .collect();
    assert_eq!(
        listed,
        vec![("NIFTY", "2026-09-29"), ("SENSEX", "2026-09-17")],
        "chains are grouped by underlying, NSE_FNO before BSE_FNO"
    );
    assert!(chains.iter().all(|c| c.contracts == 1));
}

#[test]
fn spot_is_never_subscribed_for_an_underlying_with_no_option_chain() {
    let options = vec![
        option(
            ExchangeSegment::NseFno,
            "NIFTY",
            "2026-09-29",
            2_400_000_000,
            OptionType::Call,
            ChainKind::Index,
            "nifty-ce",
        ),
        option(
            ExchangeSegment::NseFno,
            "RELIANCE",
            "2026-09-29",
            140_000_000,
            OptionType::Call,
            ChainKind::Stock,
            "reliance-ce",
        ),
    ];
    let spots = vec![
        nifty_spot(),
        spot(
            ExchangeSegment::NseEq,
            "NSE",
            "RELIANCE",
            "2885",
            SpotKind::Equity,
        ),
        spot(
            ExchangeSegment::IdxI,
            "BSE",
            "SENSEX",
            "51",
            SpotKind::Index,
        ),
        spot(ExchangeSegment::NseEq, "NSE", "TCS", "11536", SpotKind::Equity),
        spot(
            ExchangeSegment::NseEq,
            "NSE",
            "SOMESMALLCAP",
            "99999",
            SpotKind::Equity,
        ),
    ];

    let catalog = build_catalog(&master(options, spots), AS_OF).unwrap();
    let subscribed = ids(&catalog.spot);

    assert_eq!(
        subscribed,
        vec!["13"],
        "only an index underlying that carries a chain earns a spot subscription"
    );
    assert!(
        !subscribed.contains(&"2885"),
        "RELIANCE carries a stock chain, so its spot is not subscribed"
    );
    assert!(!subscribed.contains(&"51"), "SENSEX has no chain here");
    assert!(!subscribed.contains(&"11536"), "TCS has no chain here");
    assert!(
        !subscribed.contains(&"99999"),
        "a non-F&O equity must never appear"
    );
    assert_eq!(catalog.report.spot_index, 1);
}

#[test]
fn excluded_index_chains_are_reported_and_take_their_spot_with_them() {
    let options = vec![
        option(
            ExchangeSegment::BseFno,
            "SENSEX",
            "2026-09-17",
            8_460_000_000,
            OptionType::Put,
            ChainKind::Index,
            "sensex-front",
        ),
        option(
            ExchangeSegment::BseFno,
            "SENSEX50",
            "2026-09-24",
            2_680_000_000,
            OptionType::Call,
            ChainKind::Index,
            "sensex50-ce",
        ),
        option(
            ExchangeSegment::BseFno,
            "SENSEX50",
            "2026-09-24",
            2_690_000_000,
            OptionType::Put,
            ChainKind::Index,
            "sensex50-pe",
        ),
    ];
    let spots = vec![
        spot(ExchangeSegment::IdxI, "BSE", "SENSEX", "51", SpotKind::Index),
        spot(
            ExchangeSegment::IdxI,
            "BSE",
            "SENSEX50",
            "68",
            SpotKind::Index,
        ),
    ];

    let catalog = build_catalog(&master(options, spots), AS_OF).unwrap();

    assert_eq!(
        ids(&catalog.index_options),
        vec!["sensex-front"],
        "an excluded chain contributes no option instruments"
    );

    let excluded = &catalog.report.excluded_index_chains;
    assert_eq!(
        excluded.len(),
        1,
        "the exclusion must be reported, not silent"
    );
    assert_eq!(excluded[0].underlying.symbol, "SENSEX50");
    assert_eq!(excluded[0].contracts, 2);

    assert_eq!(
        ids(&catalog.spot),
        vec!["51"],
        "an excluded chain does not earn a spot subscription either"
    );
    assert_eq!(catalog.report.index_underlyings, 1);
    assert_eq!(catalog.report.spot_index, 1);
}

#[test]
fn an_extra_spot_index_is_subscribed_without_any_option_chain() {
    let options = vec![option(
        ExchangeSegment::NseFno,
        "NIFTY",
        "2026-09-29",
        2_400_000_000,
        OptionType::Call,
        ChainKind::Index,
        "nifty-ce",
    )];
    let spots = vec![
        nifty_spot(),
        spot(
            ExchangeSegment::IdxI,
            "NSE",
            "INDIA VIX",
            "21",
            SpotKind::Index,
        ),
    ];

    let catalog = build_catalog(&master(options, spots), AS_OF).unwrap();

    let spot_ids = ids(&catalog.spot);
    assert!(
        spot_ids.contains(&"21"),
        "India VIX must be subscribed even though it has no option chain"
    );
    assert!(spot_ids.contains(&"13"), "NIFTY spot stays");
    assert_eq!(spot_ids.len(), 2);
    assert!(catalog.report.missing_extra_spots.is_empty());

    assert_eq!(
        ids(&catalog.index_options),
        vec!["nifty-ce"],
        "an extra spot index never adds option contracts"
    );
    assert_eq!(
        catalog.report.index_underlyings, 1,
        "an extra spot index is not an option underlying"
    );
}

#[test]
fn a_missing_extra_spot_index_is_reported_rather_than_silently_dropped() {
    let options = vec![option(
        ExchangeSegment::NseFno,
        "NIFTY",
        "2026-09-29",
        2_400_000_000,
        OptionType::Call,
        ChainKind::Index,
        "nifty-ce",
    )];

    let catalog = build_catalog(&master(options, vec![nifty_spot()]), AS_OF).unwrap();

    assert_eq!(
        catalog.report.missing_extra_spots,
        vec!["INDIA VIX".to_owned()]
    );
    assert_eq!(ids(&catalog.spot), vec!["13"]);
}






