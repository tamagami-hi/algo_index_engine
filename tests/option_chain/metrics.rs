use super::*;
use crate::dhan_api::instruments::master::SpotKind;
use crate::dhan_api::instruments::{ExchangeSegment, to_strike_units};
use crate::option_chain::table::{Block, OptionTable, strike_step_units};

fn table(step: f64, first: f64, count: usize, spot: f64) -> OptionTable {
    let strikes: Vec<f64> = (0..count).map(|i| first + step * i as f64).collect();
    let strike_units: Vec<i64> = strikes.iter().copied().map(to_strike_units).collect();
    let size = strikes.len();

    OptionTable {
        underlying: crate::dhan_api::instruments::UnderlyingKey::new(
            ExchangeSegment::NseFno,
            "NIFTY",
        ),
        expiry: "2026-09-22".to_owned(),
        lot_size: 65,
        strike_step_units: strike_step_units(&strike_units),
        strike_units,
        strikes,
        calls: Block::zeroed(size),
        puts: Block::zeroed(size),
        spot: crate::dhan_api::instruments::SpotInstrument {
            segment: ExchangeSegment::IdxI,
            security_id: "13".to_owned(),
            kind: SpotKind::Index,
        },
        spot_price: spot,
        spot_received_at: crate::option_chain::quality::monotonic_millis(),
        spot_updates: 1,
    }
}

#[test]
fn spot_atm_rounds_to_the_nearest_strike_on_the_chains_own_step() {
    let fifty = table(50.0, 25_000.0, 11, 25_274.0);
    assert_eq!(
        spot_atm(&fifty),
        Some(25_250.0),
        "274 is nearer 250 than 300"
    );

    let mut up = fifty.clone();
    up.spot_price = 25_276.0;
    assert_eq!(spot_atm(&up), Some(25_300.0));

    let mut half = fifty.clone();
    half.spot_price = 25_275.0;
    assert_eq!(
        spot_atm(&half),
        Some(25_300.0),
        "an exact half step rounds up"
    );

    let twenty_five = table(25.0, 9_800.0, 11, 9_863.0);
    assert_eq!(
        spot_atm(&twenty_five),
        Some(9_863.0 - 863.0 % 25.0 + 25.0),
        "a 25-point chain must not be rounded on a 50-point grid"
    );

    let hundred = table(100.0, 56_200.0, 11, 56_749.0);
    assert_eq!(spot_atm(&hundred), Some(56_700.0));
    let mut hundred_up = hundred.clone();
    hundred_up.spot_price = 56_751.0;
    assert_eq!(spot_atm(&hundred_up), Some(56_800.0));
}

#[test]
fn spot_atm_is_absent_without_a_spot_price() {
    let mut chain = table(50.0, 25_000.0, 5, 0.0);
    assert_eq!(spot_atm(&chain), None);
    chain.spot_price = f64::NAN;
    assert_eq!(spot_atm(&chain), None);
}

#[test]
fn market_atm_is_the_cheapest_straddle_near_the_spot_atm() {
    let mut chain = table(50.0, 25_000.0, 11, 25_240.0);
    assert_eq!(spot_atm(&chain), Some(25_250.0));
    assert_eq!(spot_atm_index(&chain), Some(5));

    chain.calls.ltp = vec![
        300.0, 260.0, 220.0, 185.0, 150.0, 120.0, 95.0, 72.0, 55.0, 40.0, 28.0,
    ];
    chain.puts.ltp = vec![
        30.0, 42.0, 58.0, 75.0, 95.0, 118.0, 145.0, 175.0, 210.0, 250.0, 295.0,
    ];

    let straddles: Vec<f64> = chain
        .calls
        .ltp
        .iter()
        .zip(&chain.puts.ltp)
        .map(|(call, put)| call + put)
        .collect();
    let cheapest = straddles
        .iter()
        .enumerate()
        .min_by(|left, right| left.1.total_cmp(right.1))
        .map(|(index, _)| index)
        .unwrap();

    assert_eq!(market_atm_index(&chain), Some(cheapest));
    assert_eq!(metrics(&chain).market_atm, Some(chain.strikes[cheapest]));
}

#[test]
fn market_atm_ignores_strikes_missing_a_tradable_side() {
    let mut chain = table(50.0, 25_000.0, 11, 25_250.0);
    chain.calls.ltp = vec![0.0; 11];
    chain.puts.ltp = vec![0.0; 11];

    chain.calls.ltp[4] = 1.0;
    chain.puts.ltp[4] = 1.0;
    chain.calls.ltp[6] = 120.0;
    chain.puts.ltp[6] = 118.0;

    assert_eq!(
        market_atm_index(&chain),
        Some(4),
        "only two-sided strikes are candidates, and 2.0 is the cheapest"
    );

    let mut one_sided = table(50.0, 25_000.0, 11, 25_250.0);
    one_sided.calls.ltp[5] = 100.0;
    assert_eq!(
        market_atm_index(&one_sided),
        Some(5),
        "with no two-sided strike it falls back to the spot ATM row"
    );
}

#[test]
fn market_atm_search_stays_within_five_strikes_of_the_spot_atm() {
    let mut chain = table(50.0, 25_000.0, 21, 25_500.0);
    assert_eq!(spot_atm_index(&chain), Some(10));

    chain.calls.ltp = vec![100.0; 21];
    chain.puts.ltp = vec![100.0; 21];
    chain.calls.ltp[0] = 1.0;
    chain.puts.ltp[0] = 1.0;

    assert_eq!(
        market_atm_index(&chain),
        Some(5),
        "row 0 is outside the +/-5 window so the window edge wins on tie order"
    );
}

#[test]
fn max_pain_is_the_strike_where_writers_lose_least() {
    let mut chain = table(50.0, 25_000.0, 5, 25_100.0);
    chain.calls.ltp = vec![100.0; 5];
    chain.puts.ltp = vec![100.0; 5];

    chain.calls.oi = vec![0.0, 0.0, 10_000.0, 0.0, 0.0];
    chain.puts.oi = vec![0.0, 0.0, 10_000.0, 0.0, 0.0];

    let atm = market_atm_index(&chain).unwrap();
    assert_eq!(
        max_pain_index(&chain, atm),
        Some(2),
        "all open interest sits at 25100, so settling there costs writers nothing"
    );

    let mut skewed = chain.clone();
    skewed.calls.oi = vec![0.0, 0.0, 0.0, 0.0, 50_000.0];
    skewed.puts.oi = vec![50_000.0, 0.0, 0.0, 0.0, 0.0];
    let pain_low = pain_at(&skewed, 0);
    let pain_high = pain_at(&skewed, 4);
    assert!(
        pain_low.is_finite() && pain_high.is_finite(),
        "pain must be computable at both edges"
    );
}

fn pain_at(chain: &OptionTable, settle_row: usize) -> f64 {
    pain_in(chain, &(0..=chain.len() - 1), settle_row)
}

fn pain_in(chain: &OptionTable, window: &RangeInclusive<usize>, settle_row: usize) -> f64 {
    let settle = chain.strikes[settle_row];
    window
        .clone()
        .map(|row| {
            let strike = chain.strikes[row];
            if settle > strike {
                (settle - strike) * chain.calls.oi[row]
            } else if strike > settle {
                (strike - settle) * chain.puts.oi[row]
            } else {
                0.0
            }
        })
        .sum()
}

#[test]
fn max_pain_matches_a_brute_force_sweep_of_the_window() {
    let mut chain = table(100.0, 56_000.0, 21, 56_950.0);
    chain.calls.ltp = vec![50.0; 21];
    chain.puts.ltp = vec![50.0; 21];
    for row in 0..21 {
        chain.calls.oi[row] = ((row * 37) % 11) as f64 * 1_000.0;
        chain.puts.oi[row] = ((row * 53) % 13) as f64 * 1_000.0;
    }

    let atm = market_atm_index(&chain).unwrap();
    let window = clamp_window(atm, MAX_PAIN_WINDOW, MAX_PAIN_WINDOW, chain.len());
    let expected = window
        .clone()
        .min_by(|left, right| {
            pain_in(&chain, &window, *left).total_cmp(&pain_in(&chain, &window, *right))
        })
        .unwrap();

    assert_eq!(
        max_pain_index(&chain, atm),
        Some(expected),
        "candidates and the pain sum are both confined to the same window"
    );
}

#[test]
fn pcr_and_totals_ignore_non_finite_values_and_never_divide_by_zero() {
    let mut chain = table(50.0, 25_000.0, 5, 25_100.0);
    chain.calls.oi = vec![100.0, f64::NAN, 200.0, 0.0, 100.0];
    chain.puts.oi = vec![300.0, 300.0, f64::INFINITY, 0.0, 200.0];
    chain.calls.volume = vec![10.0; 5];
    chain.puts.volume = vec![25.0; 5];

    let computed = metrics(&chain);
    assert_eq!(computed.total_call_oi, 400.0);
    assert_eq!(computed.total_put_oi, 800.0);
    assert_eq!(computed.total_combined_oi, 1_200.0);
    assert_eq!(computed.pcr_oi, 2.0);
    assert_eq!(computed.pcr_volume, 2.5);

    let empty = table(50.0, 25_000.0, 5, 25_100.0);
    let zeroed = metrics(&empty);
    assert_eq!(zeroed.pcr_oi, 0.0, "no call OI must not produce infinity");
    assert_eq!(zeroed.pcr_volume, 0.0);
}

#[test]
fn synthetic_future_and_atm_straddle_read_from_the_market_atm_row() {
    let mut chain = table(50.0, 25_000.0, 11, 25_250.0);
    chain.calls.ltp = vec![0.0; 11];
    chain.puts.ltp = vec![0.0; 11];
    chain.calls.ltp[5] = 130.0;
    chain.puts.ltp[5] = 110.0;

    let computed = metrics(&chain);
    assert_eq!(computed.market_atm, Some(25_250.0));
    assert_eq!(computed.atm_straddle, Some(240.0));
    assert_eq!(
        computed.synthetic_future,
        Some(25_250.0 + 130.0 - 110.0),
        "synthetic future is strike + call - put at the market ATM"
    );
}

#[test]
fn imbalance_is_signed_and_bounded() {
    let mut chain = table(50.0, 25_000.0, 3, 25_050.0);
    chain.calls.bid_quantity[1] = 300.0;
    chain.puts.bid_quantity[1] = 100.0;
    chain.calls.ask_quantity[1] = 50.0;
    chain.puts.ask_quantity[1] = 50.0;

    assert_eq!(imbalance_at(&chain, 1), Some(0.6));
    assert_eq!(
        imbalance_at(&chain, 0),
        None,
        "a strike with no book has no imbalance rather than zero"
    );
}

#[test]
fn straddle_rows_span_ten_strikes_each_side_and_clamp_at_the_edges() {
    let mut chain = table(50.0, 25_000.0, 41, 26_000.0);
    chain.calls.ltp = vec![100.0; 41];
    chain.puts.ltp = vec![100.0; 41];
    assert_eq!(spot_atm_index(&chain), Some(20));

    chain.calls.ltp[20] = 40.0;
    chain.puts.ltp[20] = 40.0;
    let atm = market_atm_index(&chain).unwrap();
    assert_eq!(atm, 20);

    let rows = straddle_rows(&chain, atm);
    assert_eq!(
        rows.len(),
        21,
        "ten either side of the market ATM plus itself"
    );
    assert_eq!(rows[0].strike, chain.strikes[10]);
    assert_eq!(rows[20].strike, chain.strikes[30]);
    assert_eq!(rows[10].straddle_price, 80.0);

    let edge = straddle_rows(&chain, 0);
    assert_eq!(edge.len(), 11, "the window clamps at the low edge");
    let far = straddle_rows(&chain, 40);
    assert_eq!(far.len(), 11, "the window clamps at the high edge");
}

#[test]
fn market_atm_falls_back_to_the_window_edge_when_every_straddle_ties() {
    let mut chain = table(50.0, 25_000.0, 41, 26_000.0);
    chain.calls.ltp = vec![100.0; 41];
    chain.puts.ltp = vec![100.0; 41];

    assert_eq!(
        market_atm_index(&chain),
        Some(15),
        "a tie resolves to the first candidate in the window, matching the reference engine"
    );
}

#[test]
fn clamp_window_is_empty_for_an_empty_chain() {
    let window = clamp_window(0, 5, 5, 0);
    assert!(window.into_iter().next().is_none());
}
