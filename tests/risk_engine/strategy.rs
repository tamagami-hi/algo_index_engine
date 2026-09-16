use super::*;
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
        max_days_to_expiry: Some(1),
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
