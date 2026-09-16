use crate::risk_engine::strategy::{DteSelection, MAX_DTE};

fn selection(days: &[i64]) -> DteSelection {
    DteSelection::of(days.iter().copied())
}

#[test]
fn a_selection_runs_only_on_the_days_ticked() {
    let zero_and_one = selection(&[0, 1]);
    assert!(zero_and_one.allows(Some(0)));
    assert!(zero_and_one.allows(Some(1)));
    for days in [2, 3, 6, 30] {
        assert!(
            !zero_and_one.allows(Some(days)),
            "{days}DTE was not selected"
        );
    }
}

#[test]
fn a_selection_can_pick_a_single_day_or_a_gap() {
    let expiry_day_only = selection(&[0]);
    assert!(expiry_day_only.allows(Some(0)));
    assert!(!expiry_day_only.allows(Some(1)), "1DTE was not selected");

    let day_before_only = selection(&[1]);
    assert!(!day_before_only.allows(Some(0)));
    assert!(day_before_only.allows(Some(1)));

    let gapped = selection(&[0, 4]);
    assert!(gapped.allows(Some(0)));
    assert!(!gapped.allows(Some(1)));
    assert!(!gapped.allows(Some(3)));
    assert!(gapped.allows(Some(4)));
}

#[test]
fn all_dte_is_every_day_in_range_and_nothing_beyond_it() {
    let all = DteSelection::all();
    for days in 0..=MAX_DTE {
        assert!(all.allows(Some(days)), "{days}DTE is inside the range");
    }
    for days in [MAX_DTE + 1, MAX_DTE + 10, 45] {
        assert!(
            !all.allows(Some(days)),
            "{days}DTE is beyond the selectable range"
        );
    }
    assert!(all.is_all());
    assert!(!selection(&[0, 1]).is_all());
}

#[test]
fn an_expiry_already_past_never_qualifies() {
    for days in [-1, -6, -365] {
        assert!(!DteSelection::all().allows(Some(days)));
        assert!(!selection(&[0, 1]).allows(Some(days)));
        assert!(
            !selection(&[-1]).allows(Some(days)),
            "a past expiry is refused even if the stored set holds it"
        );
    }
}

#[test]
fn an_unknown_expiry_fails_closed() {
    assert!(
        !DteSelection::all().allows(None),
        "not knowing must never arm"
    );
    assert!(!selection(&[0, 1]).allows(None));
}

#[test]
fn an_empty_selection_matches_nothing() {
    let none_selected = selection(&[]);
    for days in 0..=MAX_DTE {
        assert!(!none_selected.allows(Some(days)));
    }
}

#[test]
fn the_selection_describes_itself_for_the_ui() {
    assert_eq!(DteSelection::all().describe(), "all DTE (0-6)");
    assert_eq!(selection(&[0, 1]).describe(), "0DTE, 1DTE");
    assert_eq!(selection(&[3]).describe(), "3DTE");
    assert_eq!(selection(&[]).describe(), "no DTE selected");
}

#[test]
fn the_default_is_the_zero_and_one_dte_pair() {
    assert_eq!(DteSelection::default(), selection(&[0, 1]));
}

#[test]
fn a_selection_round_trips_through_json_as_a_plain_day_list() {
    let encoded = serde_json::to_string(&selection(&[0, 2, 5])).expect("encode");
    assert_eq!(encoded, "[0,2,5]");

    for wanted in [DteSelection::all(), selection(&[0, 2, 5]), selection(&[])] {
        let encoded = serde_json::to_string(&wanted).expect("encode");
        let decoded: DteSelection = serde_json::from_str(&encoded).expect("decode");
        assert_eq!(decoded, wanted, "{encoded}");
    }

    let messy: DteSelection = serde_json::from_str("[1,0,1]").expect("decode");
    assert_eq!(
        messy,
        selection(&[0, 1]),
        "duplicates collapse, order is normalised"
    );
}


fn chain_expiring_in(days: i64) -> crate::option_chain::table::OptionTable {
    use crate::dhan_api::instruments::master::SpotKind;
    use crate::dhan_api::instruments::{
        ExchangeSegment, SpotInstrument, UnderlyingKey, days_between, ist_today, shift_iso_date,
        to_strike_units,
    };
    use crate::option_chain::table::{Block, OptionTable, strike_step_units};

    let today = ist_today().expect("today");
    let expiry = shift_iso_date(&today, days).expect("expiry");
    assert_eq!(
        days_between(&today, &expiry).expect("span"),
        days,
        "the synthetic expiry {expiry} must really be {days} days from {today}"
    );

    let strikes: Vec<f64> = (0..11).map(|index| 25_000.0 + 50.0 * index as f64).collect();
    let strike_units: Vec<i64> = strikes.iter().copied().map(to_strike_units).collect();
    let size = strikes.len();
    let mut calls = Block::zeroed(size);
    let mut puts = Block::zeroed(size);
    for row in 0..size {
        calls.security_id[row] = Some(format!("1{row}"));
        puts.security_id[row] = Some(format!("2{row}"));
        calls.ltp[row] = 100.0;
        puts.ltp[row] = 100.0;
        calls.updates[row] = 1;
        puts.updates[row] = 1;
    }

    OptionTable {
        underlying: UnderlyingKey::new(ExchangeSegment::NseFno, "NIFTY"),
        expiry,
        lot_size: 65,
        strike_step_units: strike_step_units(&strike_units),
        strike_units,
        strikes,
        calls,
        puts,
        spot: SpotInstrument {
            segment: ExchangeSegment::IdxI,
            security_id: "13".to_owned(),
            kind: SpotKind::Index,
        },
        spot_price: 25_250.0,
        spot_updates: 1,
    }
}

#[test]
fn resolving_arms_only_when_the_live_expiry_is_a_selected_dte() {
    use crate::risk_engine::resolve_strategy;
    use crate::risk_engine::strategy::Strategy;

    for chosen in 0..=MAX_DTE {
        let mut strategy = Strategy::template("NIFTY");
        strategy.id = "probe".to_owned();
        strategy.name = "probe".to_owned();
        strategy.dte = DteSelection::of([chosen]);
        assert_eq!(strategy.validate(), Ok(()));

        for actual in 0..=MAX_DTE {
            let table = chain_expiring_in(actual);
            let resolution = resolve_strategy(&strategy, &table, |_| None);
            assert_eq!(
                resolution.days_to_expiry,
                Some(actual),
                "the chain expiry must drive the DTE count"
            );
            assert_eq!(
                resolution.expiry_gate_met,
                actual == chosen,
                "{chosen}DTE selected, chain is {actual}DTE"
            );
            if actual != chosen {
                assert!(
                    !resolution.would_enter_now,
                    "an unselected DTE must never enter"
                );
            }
        }
    }
}

#[test]
fn resolving_with_all_dte_selected_arms_across_the_whole_range() {
    use crate::risk_engine::resolve_strategy;
    use crate::risk_engine::strategy::Strategy;

    let mut strategy = Strategy::template("NIFTY");
    strategy.id = "probe".to_owned();
    strategy.name = "probe".to_owned();
    strategy.dte = DteSelection::all();

    for actual in 0..=MAX_DTE {
        let table = chain_expiring_in(actual);
        let resolution = resolve_strategy(&strategy, &table, |_| None);
        assert!(
            resolution.expiry_gate_met,
            "all DTE selected, chain is {actual}DTE"
        );
        assert_eq!(resolution.dte_selection, "all DTE (0-6)");
    }
}

#[test]
fn resolving_reports_an_expiry_already_past_as_shut() {
    use crate::risk_engine::resolve_strategy;
    use crate::risk_engine::strategy::Strategy;

    let mut strategy = Strategy::template("NIFTY");
    strategy.id = "probe".to_owned();
    strategy.name = "probe".to_owned();
    strategy.dte = DteSelection::all();

    let table = chain_expiring_in(-1);
    let resolution = resolve_strategy(&strategy, &table, |_| None);
    assert_eq!(resolution.days_to_expiry, Some(-1));
    assert!(
        !resolution.expiry_gate_met,
        "a chain that already expired must not arm"
    );
    assert!(!resolution.would_enter_now);
}


#[test]
fn the_entry_window_is_one_minute_wide_and_shuts_after_it() {
    use crate::risk_engine::strategy::{ENTRY_WINDOW_MINUTES, Strategy, TimeOfDay};

    assert_eq!(
        ENTRY_WINDOW_MINUTES, 1,
        "an entry set for 09:16 may only open during 09:16"
    );

    let mut strategy = Strategy::template("NIFTY");
    strategy.id = "probe".to_owned();
    strategy.name = "probe".to_owned();
    strategy.entry_time = TimeOfDay::from_minutes(9 * 60 + 16);
    assert_eq!(strategy.validate(), Ok(()));
    assert_eq!(strategy.entry_closes_at().to_string(), "09:17");

    let at = |hours: u32, minutes: u32| hours * 60 + minutes;

    assert!(
        !strategy.entry_open(at(9, 15)),
        "09:15 is before the window"
    );
    assert!(strategy.entry_open(at(9, 16)), "09:16 is the window");
    assert!(
        !strategy.entry_open(at(9, 17)),
        "09:17 is already too late"
    );

    for (hours, minutes) in [(9, 18), (9, 30), (11, 0), (13, 45), (14, 58)] {
        assert!(
            !strategy.entry_open(at(hours, minutes)),
            "{hours:02}:{minutes:02} is a late entry and must be refused"
        );
    }
}

#[test]
fn holding_runs_past_the_entry_window_to_the_hard_exit() {
    use crate::risk_engine::strategy::{Strategy, TimeOfDay};

    let mut strategy = Strategy::template("NIFTY");
    strategy.entry_time = TimeOfDay::from_minutes(9 * 60 + 16);
    strategy.exit_time = TimeOfDay::from_minutes(14 * 60 + 59);

    let at = |hours: u32, minutes: u32| hours * 60 + minutes;

    assert!(!strategy.holdable(at(9, 15)));
    assert!(strategy.holdable(at(9, 16)));
    assert!(
        strategy.holdable(at(11, 0)),
        "a position opened at 09:16 is still managed at 11:00"
    );
    assert!(strategy.holdable(at(14, 58)));
    assert!(
        !strategy.holdable(at(14, 59)),
        "14:59 is the hard exit, not a holdable minute"
    );
    assert!(!strategy.holdable(at(15, 30)));

    assert!(
        !strategy.entry_open(at(11, 0)) && strategy.holdable(at(11, 0)),
        "holding and opening are different permissions"
    );
}

#[test]
fn resolving_refuses_an_entry_outside_the_minute_it_was_set_for() {
    use crate::dhan_api::instruments::ist_minutes_now;
    use crate::risk_engine::resolve_strategy;
    use crate::risk_engine::strategy::{Strategy, TimeOfDay};

    let now = ist_minutes_now();
    let table = chain_expiring_in(0);

    let build = |entry: u32| {
        let mut strategy = Strategy::template("NIFTY");
        strategy.id = "probe".to_owned();
        strategy.name = "probe".to_owned();
        strategy.entry_time = TimeOfDay::from_minutes(entry);
        strategy.exit_time = TimeOfDay::from_minutes(entry + 120);
        strategy
    };

    // The clock may tick between reading the minute and resolving against it, so
    // only trust an attempt that began and ended in the same minute.
    let mut entered = None;
    for _ in 0..5 {
        let before = ist_minutes_now();
        let resolution = resolve_strategy(&build(before), &table, |_| None);
        if ist_minutes_now() == before {
            entered = Some(resolution);
            break;
        }
    }
    let resolution = entered.expect("five attempts all straddled a minute boundary");
    assert!(
        resolution.entry_window_open,
        "the current minute is the entry minute"
    );
    assert!(resolution.would_enter_now, "{resolution:?}");

    // The window opened earlier and has closed: the late-start case.
    let missed = build(now.saturating_sub(30).max(1));
    let resolution = resolve_strategy(&missed, &table, |_| None);
    assert!(
        !resolution.entry_window_open,
        "a window that opened 30 minutes ago is shut"
    );
    assert!(
        !resolution.would_enter_now,
        "a late start must not enter, {resolution:?}"
    );
    assert!(
        resolution.before_hard_exit,
        "the session is still live even though entry is shut"
    );

    let too_early = build(now + 30);
    let resolution = resolve_strategy(&too_early, &table, |_| None);
    assert!(!resolution.entry_window_open, "the window has not opened");
    assert!(!resolution.would_enter_now);
}
