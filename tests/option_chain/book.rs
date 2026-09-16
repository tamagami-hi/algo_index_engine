use super::*;
use crate::dhan_api::feed::{DepthLevel, Full, Header, Message};
use crate::dhan_api::instruments::master::{
    ChainKind, InstrumentMaster, OptionContract, OptionType, SpotKind, SpotRow,
};
use crate::dhan_api::instruments::{ExchangeSegment, to_strike_units};

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


#[test]
fn the_wire_encoding_drops_f32_noise_without_moving_a_price() {
    let (master, catalog) = fixture();
    let mut book = ChainBook::build(&master, &catalog);
    book.apply(&full_message(101, 129.3, 128.8, 129.8, 120_000));
    book.apply(&full_message(102, 87.45, 87.0, 87.9, 95_000));

    let columns = book.columns("NIFTY").expect("columns");
    let encoded = serde_json::to_string(&columns).expect("encode");

    assert!(
        encoded.contains("\"ltp\":[129.3,0,0]"),
        "the exchange sent 129.3 and the wire must say 129.3, not the f32 widening: {encoded}"
    );
    assert!(
        !encoded.contains("129.3000"),
        "no f32 noise may reach the browser: {encoded}"
    );
    assert!(
        encoded.contains("\"bid_quantity\":[750,0,0]"),
        "an integral quantity carries no decimal point: {encoded}"
    );
    assert!(
        encoded.contains("\"strike\":[25000,25050,25100]"),
        "strikes are whole numbers: {encoded}"
    );
    assert!(
        encoded.contains("\"ltp\":[87.45,0,0]"),
        "two decimals of premium survive: {encoded}"
    );

    let decoded: serde_json::Value = serde_json::from_str(&encoded).expect("decode");
    let ltp = &decoded["call"]["ltp"][0];
    assert_eq!(
        ltp.as_f64().expect("a number"),
        129.3,
        "the value a client reads back is the price the exchange sent"
    );
}

#[test]
fn a_non_finite_cell_is_null_rather_than_a_fabricated_zero() {
    let (master, catalog) = fixture();
    let mut book = ChainBook::build(&master, &catalog);
    book.apply(&full_message(101, f32::NAN, 1.0, 2.0, 10));

    let columns = book.columns("NIFTY").expect("columns");
    let encoded = serde_json::to_string(&columns).expect("a non-finite cell must still encode");
    assert!(
        encoded.contains("\"ltp\":[null,0,0]"),
        "an unusable price is absent, not zero: {encoded}"
    );
}

#[test]
#[ignore]
fn perf_probe() {
    use crate::server::EngineState;
    use std::time::Instant;

    const SYMBOLS: [(&str, f64, f64, u32); 7] = [
        ("NIFTY", 25_000.0, 50.0, 65),
        ("BANKNIFTY", 54_000.0, 100.0, 30),
        ("FINNIFTY", 25_500.0, 50.0, 25),
        ("MIDCPNIFTY", 12_800.0, 25.0, 120),
        ("NIFTYNXT50", 68_000.0, 100.0, 60),
        ("SENSEX", 82_000.0, 100.0, 20),
        ("BANKEX", 62_000.0, 100.0, 30),
    ];
    const STRIKES: usize = 260;

    let mut options = Vec::with_capacity(SYMBOLS.len() * STRIKES * 2);
    let mut spots = Vec::new();
    let mut security_id = 10_000_i32;
    let mut option_ids: Vec<i32> = Vec::new();

    for (symbol, base, step, lot) in SYMBOLS {
        let segment = if symbol == "SENSEX" || symbol == "BANKEX" {
            ExchangeSegment::BseFno
        } else {
            ExchangeSegment::NseFno
        };
        for index in 0..STRIKES {
            let strike = base + step * (index as f64 - STRIKES as f64 / 2.0);
            for option_type in [OptionType::Call, OptionType::Put] {
                security_id += 1;
                option_ids.push(security_id);
                options.push(OptionContract {
                    segment,
                    security_id: security_id.to_string(),
                    underlying_symbol: symbol.to_owned(),
                    expiry: "2026-09-22".to_owned(),
                    strike_units: to_strike_units(strike),
                    option_type,
                    lot_size: lot,
                    kind: ChainKind::Index,
                });
            }
        }
        security_id += 1;
        spots.push(SpotRow {
            segment: ExchangeSegment::IdxI,
            security_id: security_id.to_string(),
            exchange_id: "NSE".to_owned(),
            underlying_symbol: symbol.to_owned(),
            symbol_name: symbol.to_owned(),
            expiry: String::new(),
            kind: SpotKind::Index,
        });
    }

    let master = InstrumentMaster {
        options,
        spots,
        report: Default::default(),
    };
    let catalog =
        crate::dhan_api::instruments::build_catalog(&master, "2026-09-16").expect("catalog");
    let book = ChainBook::build(&master, &catalog);

    println!(
        "\nuniverse: {} chains, {} option contracts, {} strikes each",
        book.chains(),
        master.options.len(),
        STRIKES
    );

    let engine = EngineState::new();
    engine.set_book(book);

    let packets_per_frame = 10;
    let frames = 2_000;
    let mut cursor = 0usize;

    let mut batches: Vec<Vec<Message>> = Vec::with_capacity(frames);
    for _ in 0..frames {
        let mut batch = Vec::with_capacity(packets_per_frame);
        for _ in 0..packets_per_frame {
            let id = option_ids[cursor % option_ids.len()];
            cursor += 1;
            let price = 80.0 + (cursor % 50) as f32;
            batch.push(full_message(id, price, price - 0.5, price + 0.5, 120_000));
        }
        batches.push(batch);
    }

    let started = Instant::now();
    for batch in &batches {
        engine.apply_frame(1_620, batch);
    }
    let elapsed = started.elapsed();
    println!(
        "apply_frame        {:>8.1} us/frame   ({} frames x {} packets, {:.0} frames/s)",
        elapsed.as_secs_f64() * 1e6 / frames as f64,
        frames,
        packets_per_frame,
        frames as f64 / elapsed.as_secs_f64()
    );

    let columns_started = Instant::now();
    let rounds = 500;
    let mut bytes = 0usize;
    for _ in 0..rounds {
        let columns = engine.chain_columns("NIFTY").expect("columns");
        bytes = serde_json::to_string(&columns).expect("encode").len();
    }
    let columns_elapsed = columns_started.elapsed();
    println!(
        "chain_columns+json {:>8.1} us/call    ({} bytes per payload)",
        columns_elapsed.as_secs_f64() * 1e6 / rounds as f64,
        bytes
    );

    let build_started = Instant::now();
    for _ in 0..rounds {
        let columns = engine.chain_columns("NIFTY").expect("columns");
        std::hint::black_box(&columns);
    }
    let build_elapsed = build_started.elapsed();
    println!(
        "  chain_columns    {:>8.1} us/call    (metrics + 19 vec clones)",
        build_elapsed.as_secs_f64() * 1e6 / rounds as f64
    );

    let prebuilt = engine.chain_columns("NIFTY").expect("columns");
    let json_started = Instant::now();
    for _ in 0..rounds {
        std::hint::black_box(serde_json::to_string(&prebuilt).expect("encode"));
    }
    let json_elapsed = json_started.elapsed();
    println!(
        "  serde_json only  {:>8.1} us/call",
        json_elapsed.as_secs_f64() * 1e6 / rounds as f64
    );

    let snapshot_started = Instant::now();
    for _ in 0..rounds {
        let snapshot = engine.snapshot();
        bytes = serde_json::to_string(&snapshot).expect("encode").len();
    }
    let snapshot_elapsed = snapshot_started.elapsed();
    println!(
        "snapshot+json      {:>8.1} us/call    ({} bytes per payload)",
        snapshot_elapsed.as_secs_f64() * 1e6 / rounds as f64,
        bytes
    );

    let metrics_started = Instant::now();
    for _ in 0..rounds {
        let all = engine.chain_metrics();
        std::hint::black_box(&all);
    }
    let metrics_elapsed = metrics_started.elapsed();
    println!(
        "chain_metrics(all) {:>8.1} us/call    (recomputed inside every apply_frame)\n",
        metrics_elapsed.as_secs_f64() * 1e6 / rounds as f64
    );
}
