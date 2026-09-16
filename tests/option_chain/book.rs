use super::*;
use crate::dhan_api::feed::{DepthLevel, Full, Header, Message};
use crate::dhan_api::instruments::{
    ChainKind, ExchangeSegment, InstrumentMaster, OptionContract, OptionType, SpotKind, SpotRow,
    to_strike_units,
};

fn option(strike: f64, option_type: OptionType, security_id: &str) -> OptionContract {
    OptionContract {
        segment: ExchangeSegment::NseFno,
        security_id: security_id.to_owned(),
        underlying_symbol: "NIFTY".to_owned(),
        expiry: "2026-09-22".to_owned(),
        strike_units: to_strike_units(strike),
        option_type,
        lot_size: 65,
        kind: ChainKind::Index,
    }
}

fn index_row(symbol: &str, security_id: &str) -> SpotRow {
    SpotRow {
        segment: ExchangeSegment::IdxI,
        security_id: security_id.to_owned(),
        exchange_id: "NSE".to_owned(),
        underlying_symbol: symbol.to_owned(),
        symbol_name: symbol.to_owned(),
        expiry: String::new(),
        kind: SpotKind::Index,
    }
}

fn fixture() -> (InstrumentMaster, crate::dhan_api::instruments::Catalog) {
    let master = InstrumentMaster {
        options: vec![
            option(25_000.0, OptionType::Call, "101"),
            option(25_000.0, OptionType::Put, "102"),
            option(25_050.0, OptionType::Call, "103"),
            option(25_050.0, OptionType::Put, "104"),
            option(25_100.0, OptionType::Call, "105"),
            option(25_100.0, OptionType::Put, "106"),
        ],
        spots: vec![index_row("NIFTY", "13"), index_row("INDIA VIX", "21")],
        report: Default::default(),
    };
    let catalog =
        crate::dhan_api::instruments::build_catalog(&master, "2026-09-16").expect("catalog");
    (master, catalog)
}

fn full_message(security_id: i32, ltp: f32, bid: f32, ask: f32, oi: i32) -> Message {
    let mut depth = [DepthLevel::default(); 5];
    depth[0] = DepthLevel {
        bid_quantity: 750,
        ask_quantity: 900,
        bid_orders: 3,
        ask_orders: 4,
        bid_price: bid,
        ask_price: ask,
    };
    Message {
        header: Header {
            code: crate::dhan_api::feed::CODE_FULL,
            declared_len: 162,
            segment: Some(ExchangeSegment::NseFno),
            security_id,
        },
        packet: Packet::Full(Full {
            last_price: ltp,
            volume: 4_200,
            open_interest: oi,
            depth,
            ..Full::default()
        }),
    }
}

fn index_message(security_id: i32, price: f32) -> Message {
    Message {
        header: Header {
            code: crate::dhan_api::feed::CODE_INDEX,
            declared_len: 16,
            segment: Some(ExchangeSegment::IdxI),
            security_id,
        },
        packet: Packet::Index { last_price: price },
    }
}

#[test]
fn a_full_packet_lands_on_the_right_chain_side_and_strike() {
    let (master, catalog) = fixture();
    let mut book = ChainBook::build(&master, &catalog);
    assert_eq!(book.chains(), 1);

    book.apply(&full_message(103, 120.5, 120.0, 121.0, 55_000));

    let view = book.view("NIFTY").expect("chain view");
    let row = &view.rows[1];
    assert_eq!(row.strike, 25_050.0);

    let call = row.call.expect("the call side must be quoted");
    assert_eq!(call.ltp, 120.5);
    assert_eq!(call.bid, 120.0);
    assert_eq!(call.ask, 121.0);
    assert_eq!(call.bid_quantity, 750.0);
    assert_eq!(call.ask_quantity, 900.0);
    assert_eq!(call.oi, 55_000.0);
    assert_eq!(call.volume, 4_200.0);

    let put = row.put.expect("the put row exists");
    assert_eq!(put.ltp, 0.0, "the put leg was never quoted");

    assert_eq!(view.rows[0].call.unwrap().ltp, 0.0);
    assert_eq!(book.stats().0, 1);
    assert_eq!(book.stats().1, 0);
}

#[test]
fn a_spot_packet_sets_the_chain_spot_and_a_reference_index_is_tracked_separately() {
    let (master, catalog) = fixture();
    let mut book = ChainBook::build(&master, &catalog);

    book.apply(&index_message(13, 25_063.4));
    book.apply(&index_message(21, 11.85));

    let view = book.view("NIFTY").expect("chain view");
    assert_eq!(view.metrics.spot_price, 25_063.4_f32 as f64);
    assert_eq!(
        view.metrics.spot_atm,
        Some(25_050.0),
        "25063 rounds down on a 50-point chain"
    );

    assert_eq!(book.stats().2.get("INDIA VIX").copied(), Some(11.85_f32 as f64));
    assert_eq!(
        book.stats().2.get("NIFTY").copied(),
        Some(25_063.4_f32 as f64),
        "the chain spot is also a labelled reference"
    );
}

#[test]
fn an_unknown_security_is_counted_as_unmatched_rather_than_misapplied() {
    let (master, catalog) = fixture();
    let mut book = ChainBook::build(&master, &catalog);

    book.apply(&full_message(999_999, 1.0, 1.0, 2.0, 1));

    assert_eq!(book.stats().0, 0);
    assert_eq!(book.stats().1, 1);
    let view = book.view("NIFTY").unwrap();
    assert!(view.rows.iter().all(|row| row.call.unwrap().ltp == 0.0));
}

#[test]
fn previous_close_and_open_interest_drive_the_change_columns() {
    let (master, catalog) = fixture();
    let mut book = ChainBook::build(&master, &catalog);

    book.apply(&Message {
        header: Header {
            code: crate::dhan_api::feed::CODE_PREV_CLOSE,
            declared_len: 16,
            segment: Some(ExchangeSegment::NseFno),
            security_id: 101,
        },
        packet: Packet::PrevClose {
            close: 100.0,
            open_interest: 40_000,
        },
    });
    book.apply(&full_message(101, 130.0, 129.5, 130.5, 52_000));

    let view = book.view("NIFTY").unwrap();
    let call = view.rows[0].call.unwrap();
    assert_eq!(call.change, 30.0, "130 last against a 100 previous close");
    assert_eq!(call.change_in_oi, 12_000.0);
}

#[test]
fn metrics_report_only_quoted_strikes() {
    let (master, catalog) = fixture();
    let mut book = ChainBook::build(&master, &catalog);

    assert_eq!(book.metrics()[0].quoted_strikes, 0);

    book.apply(&full_message(101, 130.0, 129.0, 131.0, 1_000));
    book.apply(&full_message(106, 140.0, 139.0, 141.0, 2_000));

    let reported = &book.metrics()[0];
    assert_eq!(reported.strikes, 3);
    assert_eq!(reported.quoted_strikes, 2);
    assert_eq!(reported.symbol, "NIFTY");
    assert_eq!(reported.expiry, "2026-09-22");
    assert_eq!(reported.strike_step, 50.0);
}

#[test]
fn an_unknown_symbol_has_no_view() {
    let (master, catalog) = fixture();
    let book = ChainBook::build(&master, &catalog);
    assert!(book.view("BANKNIFTY").is_none());
    assert!(book.view("nifty").is_some(), "lookup is case-insensitive");
}
