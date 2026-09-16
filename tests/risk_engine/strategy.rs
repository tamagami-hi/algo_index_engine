use super::*;
use crate::risk_engine::strategy::{DteSelection, MAX_DTE};
use crate::risk_engine::strike::{Moneyness, StrikeCriteria};

fn short_leg(side: Side) -> LegDefinition {
    LegDefinition {
        side,
        action: Action::Sell,
        lots: 1,
        strike: StrikeCriteria::Relative {
            moneyness: Moneyness::Atm,
            steps: 0,
        },
        stop_loss: Some(Threshold {
            method: RiskMethod::Percent,
            value: 75.0,
        }),
        target: None,
        trailing: None,
    }
}

fn straddle() -> Strategy {
    Strategy {
        id: "nifty-atm-straddle".to_owned(),
        name: "NIFTY ATM short straddle".to_owned(),
        notes: String::new(),
        underlying: "NIFTY".to_owned(),
        entry_time: TimeOfDay::from_minutes(9 * 60 + 16),
        exit_time: TimeOfDay::from_minutes(14 * 60 + 59),
        dte: DteSelection::default(),
        entry_condition: EntryCondition::ReferenceAbove {
            reference: "INDIA VIX".to_owned(),
            value: 12.0,
        },
        legs: vec![short_leg(Side::Call), short_leg(Side::Put)],
        overall: OverallRisk::default(),
        loss_coverage: LossCoverage::Breakeven,
    }
}

#[test]
fn a_well_formed_strategy_validates() {
    assert_eq!(straddle().validate(), Ok(()));
}

#[test]
fn time_of_day_round_trips_as_hh_mm() {
    let encoded = serde_json::to_string(&straddle()).expect("encode");
    assert!(encoded.contains("\"entry_time\":\"09:16\""));
    assert!(encoded.contains("\"exit_time\":\"14:59\""));

    let decoded: Strategy = serde_json::from_str(&encoded).expect("decode");
    assert_eq!(decoded.entry_time.minutes(), 556);
    assert_eq!(decoded.exit_time.minutes(), 899);
}

#[test]
fn a_bad_clock_string_is_rejected_rather_than_defaulted() {
    let encoded = serde_json::to_string(&straddle()).unwrap();
    for bad in ["25:00", "09:60", "morning", "9"] {
        let broken = encoded.replace("\"09:16\"", &format!("\"{bad}\""));
        assert!(
            serde_json::from_str::<Strategy>(&broken).is_err(),
            "{bad} must not parse as a time"
        );
    }
}

#[test]
fn an_exit_at_or_before_entry_is_rejected() {
    let mut strategy = straddle();
    strategy.exit_time = strategy.entry_time;
    assert!(matches!(
        strategy.validate(),
        Err(StrategyError::ExitNotAfterEntry { .. })
    ));

    strategy.exit_time = TimeOfDay::from_minutes(9 * 60);
    assert!(matches!(
        strategy.validate(),
        Err(StrategyError::ExitNotAfterEntry { .. })
    ));
}

#[test]
fn an_id_that_would_escape_the_strategy_directory_is_rejected() {
    for bad in ["../secrets", "a/b", "with space", "dot.dot"] {
        let mut strategy = straddle();
        strategy.id = bad.to_owned();
        assert!(
            matches!(strategy.validate(), Err(StrategyError::IdNotSlug { .. })),
            "{bad} must not be accepted as an id"
        );
    }
}

#[test]
fn a_strategy_with_no_dte_selected_is_rejected() {
    let mut strategy = straddle();
    strategy.dte = DteSelection::of([]);
    assert_eq!(strategy.validate(), Err(StrategyError::NoDteSelected));
}

#[test]
fn a_dte_outside_the_selectable_range_is_rejected() {
    for day in [-1, MAX_DTE + 1, 400] {
        let mut strategy = straddle();
        strategy.dte = DteSelection::of([0, day]);
        assert_eq!(
            strategy.validate(),
            Err(StrategyError::DteOutOfRange { day, max: MAX_DTE }),
            "{day}DTE is not selectable"
        );
    }

    let mut every_day = straddle();
    every_day.dte = DteSelection::all();
    assert_eq!(every_day.validate(), Ok(()));
}

#[test]
fn a_strategy_needs_legs_and_a_nonzero_size() {
    let mut empty = straddle();
    empty.legs.clear();
    assert_eq!(empty.validate(), Err(StrategyError::NoLegs));

    let mut sizeless = straddle();
    sizeless.legs[1].lots = 0;
    assert_eq!(sizeless.validate(), Err(StrategyError::ZeroLots { leg: 1 }));
}

#[test]
fn percentage_and_point_thresholds_convert_against_the_entry_premium() {
    let percent = Threshold {
        method: RiskMethod::Percent,
        value: 75.0,
    };
    assert_eq!(percent.points_from(120.0), 90.0);

    let points = Threshold {
        method: RiskMethod::Points,
        value: 75.0,
    };
    assert_eq!(
        points.points_from(120.0),
        75.0,
        "a point threshold ignores the entry premium"
    );
}

#[test]
fn the_vix_gate_selects_between_two_saved_strategies() {
    let high_vix = straddle();
    let mut low_vix = straddle();
    low_vix.id = "nifty-itm3-guts".to_owned();
    low_vix.entry_condition = EntryCondition::ReferenceAtMost {
        reference: "INDIA VIX".to_owned(),
        value: 12.0,
    };

    let at = |value: f64| move |key: &str| (key == "INDIA VIX").then_some(value);

    assert!(high_vix.entry_condition.is_met(at(14.0)));
    assert!(!low_vix.entry_condition.is_met(at(14.0)));

    assert!(!high_vix.entry_condition.is_met(at(11.0)));
    assert!(low_vix.entry_condition.is_met(at(11.0)));

    assert!(
        !high_vix.entry_condition.is_met(at(12.0)),
        "12 is not above 12"
    );
    assert!(low_vix.entry_condition.is_met(at(12.0)), "12 is at most 12");
}

#[test]
fn a_missing_reference_never_fires_an_entry() {
    let strategy = straddle();
    assert!(
        !strategy.entry_condition.is_met(|_| None),
        "an absent VIX must not be read as a satisfied condition"
    );
    assert!(
        EntryCondition::Always.is_met(|_| None),
        "an unconditional entry does not need a reference"
    );
}


fn percent(value: f64) -> Threshold {
    Threshold {
        method: RiskMethod::Percent,
        value,
    }
}

fn points(value: f64) -> Threshold {
    Threshold {
        method: RiskMethod::Points,
        value,
    }
}

fn trail(arm_at: Threshold, give_back: Threshold) -> TrailingRule {
    TrailingRule {
        arm_at,
        give_back,
        start_after_minutes: None,
    }
}

#[test]
fn a_short_leg_cannot_target_more_profit_than_the_premium_it_collected() {
    let mut strategy = straddle();
    strategy.legs[0].target = Some(percent(100.0));
    assert_eq!(
        strategy.validate(),
        Ok(()),
        "the whole premium is reachable: the option can decay to zero"
    );

    for beyond in [100.01, 120.0, 200.0] {
        let mut strategy = straddle();
        strategy.legs[0].target = Some(percent(beyond));
        assert_eq!(
            strategy.validate(),
            Err(StrategyError::BeyondPremiumCeiling {
                leg: Some(0),
                field: "target",
                value: beyond,
                ceiling: 100.0,
            }),
            "{beyond}% profit on a short can never be reached"
        );
    }
}

#[test]
fn a_short_legs_profit_trail_is_bounded_by_the_premium_too() {
    let mut arming = straddle();
    arming.legs[1].trailing = Some(trail(percent(140.0), percent(10.0)));
    assert_eq!(
        arming.validate(),
        Err(StrategyError::BeyondPremiumCeiling {
            leg: Some(1),
            field: "trailing.arm_at",
            value: 140.0,
            ceiling: 100.0,
        }),
        "a trail that arms past the premium never arms"
    );

    let mut giving_back = straddle();
    giving_back.legs[1].trailing = Some(trail(percent(60.0), percent(101.0)));
    assert!(
        matches!(
            giving_back.validate(),
            Err(StrategyError::BeyondPremiumCeiling { .. })
                | Err(StrategyError::TrailGivesBackMoreThanItCaptures { .. })
        ),
        "giving back more than the premium is not a trail"
    );

    let mut sensible = straddle();
    sensible.legs[1].trailing = Some(trail(percent(40.0), percent(15.0)));
    assert_eq!(sensible.validate(), Ok(()));
}

#[test]
fn a_short_legs_stop_loss_is_not_capped_because_a_short_can_lose_more_than_it_took() {
    let mut strategy = straddle();
    strategy.legs[0].stop_loss = Some(percent(300.0));
    assert_eq!(
        strategy.validate(),
        Ok(()),
        "a premium of 100 running to 400 is a 300% loss and entirely possible"
    );
}

#[test]
fn a_long_legs_stop_loss_is_capped_because_it_cannot_lose_more_than_it_paid() {
    let mut strategy = straddle();
    strategy.legs[0].action = Action::Buy;
    strategy.legs[0].stop_loss = Some(percent(100.0));
    assert_eq!(
        strategy.validate(),
        Ok(()),
        "the whole premium paid can be lost"
    );

    strategy.legs[0].stop_loss = Some(percent(150.0));
    assert_eq!(
        strategy.validate(),
        Err(StrategyError::BeyondPremiumCeiling {
            leg: Some(0),
            field: "stop_loss",
            value: 150.0,
            ceiling: 100.0,
        })
    );

    let mut upside = straddle();
    upside.legs[0].action = Action::Buy;
    upside.legs[0].target = Some(percent(400.0));
    assert_eq!(
        upside.validate(),
        Ok(()),
        "a long option's upside has no premium ceiling"
    );
}

#[test]
fn a_point_threshold_is_never_measured_against_the_percent_ceiling() {
    let mut strategy = straddle();
    strategy.legs[0].target = Some(points(250.0));
    assert_eq!(
        strategy.validate(),
        Ok(()),
        "250 points is not 250 percent and the ceiling does not apply"
    );
}

#[test]
fn a_trail_may_not_give_back_more_than_it_captured() {
    let mut strategy = straddle();
    strategy.legs[0].trailing = Some(trail(percent(40.0), percent(60.0)));
    assert_eq!(
        strategy.validate(),
        Err(StrategyError::TrailGivesBackMoreThanItCaptures {
            leg: Some(0),
            arm_at: 40.0,
            give_back: 60.0,
        }),
        "arming at 40 and surrendering 60 puts the stop below the entry"
    );

    strategy.legs[0].trailing = Some(trail(percent(40.0), percent(40.0)));
    assert_eq!(
        strategy.validate(),
        Ok(()),
        "giving back exactly what was captured trails to breakeven"
    );

    let mut mixed = straddle();
    mixed.legs[0].trailing = Some(trail(percent(40.0), points(60.0)));
    assert_eq!(
        mixed.validate(),
        Ok(()),
        "percent against points cannot be compared without a premium"
    );
}

#[test]
fn the_whole_positions_profit_is_capped_only_when_every_leg_sold_premium() {
    let mut all_short = straddle();
    all_short.overall.target = Some(percent(101.0));
    assert_eq!(
        all_short.validate(),
        Err(StrategyError::BeyondPremiumCeiling {
            leg: None,
            field: "overall.target",
            value: 101.0,
            ceiling: 100.0,
        })
    );

    all_short.overall.target = Some(percent(60.0));
    all_short.overall.trailing = Some(trail(percent(45.0), percent(12.0)));
    assert_eq!(all_short.validate(), Ok(()));

    let mut mixed = straddle();
    mixed.legs[0].action = Action::Buy;
    mixed.overall.target = Some(percent(150.0));
    assert_eq!(
        mixed.validate(),
        Ok(()),
        "a position holding a long leg has no premium ceiling on its profit"
    );
}

#[test]
fn a_threshold_of_zero_is_rejected_wherever_it_appears() {
    for build in [
        (|strategy: &mut Strategy| strategy.legs[0].stop_loss = Some(percent(0.0)))
            as fn(&mut Strategy),
        |strategy: &mut Strategy| strategy.legs[0].target = Some(percent(0.0)),
        |strategy: &mut Strategy| {
            strategy.legs[0].trailing = Some(trail(percent(0.0), percent(0.0)));
        },
    ] {
        let mut strategy = straddle();
        build(&mut strategy);
        assert!(
            matches!(
                strategy.validate(),
                Err(StrategyError::NonPositiveThreshold { leg: Some(0), .. })
            ),
            "a zero threshold is not a rule"
        );
    }

    let mut daily = straddle();
    daily.overall.daily_loss_limit = Some(0.0);
    assert_eq!(
        daily.validate(),
        Err(StrategyError::NonPositiveThreshold {
            leg: None,
            field: "daily_loss_limit",
        })
    );
}
