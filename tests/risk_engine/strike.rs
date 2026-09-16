use super::*;
use crate::dhan_api::instruments::{ExchangeSegment, SpotKind, UnderlyingKey};
use crate::option_chain::table::strike_step_units;

fn chain(step: f64, first: f64, count: usize, spot: f64) -> OptionTable {
    let strikes: Vec<f64> = (0..count).map(|i| first + step * i as f64).collect();
    let strike_units: Vec<i64> = strikes.iter().copied().map(to_strike_units).collect();
    let size = strikes.len();

    let mut table = OptionTable {
        underlying: UnderlyingKey::new(ExchangeSegment::NseFno, "NIFTY"),
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
        spot_updates: 1,
    };
    for row in 0..size {
        table.calls.security_id[row] = Some(format!("c{row}"));
        table.puts.security_id[row] = Some(format!("p{row}"));
    }
    table
}

#[test]
fn atm_resolves_to_the_spot_rounded_strike_on_the_chain_step() {
    let table = chain(50.0, 25_000.0, 11, 25_262.0);
    let call = resolve(
        &table,
        Side::Call,
        &StrikeCriteria::Relative {
            moneyness: Moneyness::Atm,
            steps: 0,
        },
    )
    .expect("atm call");
    assert_eq!(call.strike, 25_250.0);

    let put = resolve(
        &table,
        Side::Put,
        &StrikeCriteria::Relative {
            moneyness: Moneyness::Atm,
            steps: 0,
        },
    )
    .expect("atm put");
    assert_eq!(put.strike, 25_250.0, "both sides share the ATM strike");
}

#[test]
fn itm3_puts_the_call_below_atm_and_the_put_above_it() {
    let table = chain(50.0, 25_000.0, 21, 25_500.0);
    let itm3 = StrikeCriteria::Relative {
        moneyness: Moneyness::Itm,
        steps: 3,
    };

    let call = resolve(&table, Side::Call, &itm3).expect("itm3 call");
    let put = resolve(&table, Side::Put, &itm3).expect("itm3 put");

    assert_eq!(call.strike, 25_350.0, "a call is in the money below spot");
    assert_eq!(put.strike, 25_650.0, "a put is in the money above spot");
    assert!(
        call.strike < put.strike,
        "call strike beneath put strike is a short guts, not a strangle"
    );
}

#[test]
fn otm3_mirrors_itm3() {
    let table = chain(50.0, 25_000.0, 21, 25_500.0);
    let otm3 = StrikeCriteria::Relative {
        moneyness: Moneyness::Otm,
        steps: 3,
    };

    assert_eq!(
        resolve(&table, Side::Call, &otm3).unwrap().strike,
        25_650.0
    );
    assert_eq!(resolve(&table, Side::Put, &otm3).unwrap().strike, 25_350.0);
}

#[test]
fn relative_selection_uses_the_chains_own_step_not_a_hardcoded_fifty() {
    let midcap = chain(25.0, 9_800.0, 21, 10_050.0);
    let itm2 = StrikeCriteria::Relative {
        moneyness: Moneyness::Itm,
        steps: 2,
    };
    assert_eq!(
        resolve(&midcap, Side::Call, &itm2).unwrap().strike,
        10_000.0,
        "two 25-point steps below a 10050 ATM"
    );

    let banknifty = chain(100.0, 56_000.0, 21, 56_950.0);
    assert_eq!(
        resolve(&banknifty, Side::Call, &itm2).unwrap().strike,
        56_800.0,
        "two 100-point steps below a 57000 ATM"
    );
}

#[test]
fn a_strike_outside_the_chain_is_an_error_not_a_clamp() {
    let table = chain(50.0, 25_000.0, 5, 25_100.0);
    let far = StrikeCriteria::Relative {
        moneyness: Moneyness::Itm,
        steps: 40,
    };
    assert!(matches!(
        resolve(&table, Side::Call, &far),
        Err(StrikeError::StrikeOutsideChain { .. })
    ));
}

#[test]
fn relative_selection_needs_a_spot_price() {
    let table = chain(50.0, 25_000.0, 5, 0.0);
    assert_eq!(
        resolve(
            &table,
            Side::Call,
            &StrikeCriteria::Relative {
                moneyness: Moneyness::Atm,
                steps: 0
            }
        ),
        Err(StrikeError::NoSpotPrice)
    );
}

#[test]
fn closest_premium_ignores_unquoted_strikes() {
    let mut table = chain(50.0, 25_000.0, 5, 25_100.0);
    table.calls.ltp = vec![0.0, 0.0, 140.0, 90.0, 0.0];

    let resolved = resolve(
        &table,
        Side::Call,
        &StrikeCriteria::ClosestPremium { target: 10.0 },
    )
    .expect("closest premium");
    assert_eq!(
        resolved.strike, 25_150.0,
        "a zero premium is not a tradable candidate"
    );
    assert_eq!(resolved.ltp, 90.0);
}

#[test]
fn premium_at_least_takes_the_smallest_premium_that_still_clears_the_threshold() {
    let mut table = chain(50.0, 25_000.0, 9, 25_200.0);
    table.calls.ltp = vec![
        400.0, 330.0, 260.0, 200.0, 150.0, 110.0, 75.0, 48.0, 30.0,
    ];
    table.puts.ltp = vec![30.0, 48.0, 75.0, 110.0, 150.0, 200.0, 260.0, 330.0, 400.0];

    let resolved = resolve(
        &table,
        Side::Call,
        &StrikeCriteria::PremiumAtLeast { threshold: 100.0 },
    )
    .expect("premium at least");
    assert_eq!(
        resolved.ltp, 110.0,
        "110 is the least premium at or above 100, i.e. the furthest strike still paying it"
    );
}

#[test]
fn premium_at_most_takes_the_largest_premium_within_the_cap() {
    let mut table = chain(50.0, 25_000.0, 9, 25_200.0);
    table.calls.ltp = vec![
        400.0, 330.0, 260.0, 200.0, 150.0, 110.0, 75.0, 48.0, 30.0,
    ];
    table.puts.ltp = vec![30.0, 48.0, 75.0, 110.0, 150.0, 200.0, 260.0, 330.0, 400.0];

    let resolved = resolve(
        &table,
        Side::Call,
        &StrikeCriteria::PremiumAtMost { threshold: 100.0 },
    )
    .expect("premium at most");
    assert_eq!(resolved.ltp, 75.0);
}

#[test]
fn premium_range_rejects_a_window_no_strike_satisfies() {
    let mut table = chain(50.0, 25_000.0, 5, 25_100.0);
    table.calls.ltp = vec![300.0, 250.0, 200.0, 150.0, 100.0];

    assert_eq!(
        resolve(
            &table,
            Side::Call,
            &StrikeCriteria::PremiumRange {
                lower: 10.0,
                upper: 20.0
            }
        ),
        Err(StrikeError::NoStrikeMeetsCriteria)
    );
}

#[test]
fn straddle_width_steps_away_from_the_money_in_the_right_direction() {
    let mut table = chain(50.0, 25_000.0, 41, 26_000.0);
    table.calls.ltp = vec![100.0; 41];
    table.puts.ltp = vec![100.0; 41];
    table.calls.ltp[20] = 60.0;
    table.puts.ltp[20] = 60.0;

    let away = StrikeCriteria::StraddleWidth {
        multiplier: 1.0,
        away_from_atm: true,
    };
    let call = resolve(&table, Side::Call, &away).expect("call side");
    let put = resolve(&table, Side::Put, &away).expect("put side");

    assert!(call.strike > 26_000.0, "a call moves up away from the money");
    assert!(put.strike < 26_000.0, "a put moves down away from the money");
    assert_eq!(call.strike, 26_100.0, "120 of straddle rounds to two steps");
    assert_eq!(put.strike, 25_900.0);
}

#[test]
fn a_strike_with_no_contract_on_that_side_is_reported() {
    let mut table = chain(50.0, 25_000.0, 5, 25_100.0);
    table.calls.security_id[2] = None;

    assert!(matches!(
        resolve(
            &table,
            Side::Call,
            &StrikeCriteria::Relative {
                moneyness: Moneyness::Atm,
                steps: 0
            }
        ),
        Err(StrikeError::NoContractAtStrike { .. })
    ));
}
