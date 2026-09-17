use super::*;
use crate::risk_engine::strategy::{
    Action, EntryCondition, LegDefinition, LossCoverage, OverallRisk, RiskMethod, Threshold,
    TimeOfDay,
};
use crate::risk_engine::strike::{Moneyness, Side, StrikeCriteria};

use crate::config::sandbox::Sandbox;

fn strategy(id: &str, underlying: &str) -> Strategy {
    Strategy {
        id: id.to_owned(),
        name: format!("{underlying} short straddle"),
        notes: String::new(),
        underlying: underlying.to_owned(),
        entry_time: TimeOfDay::from_minutes(9 * 60 + 16),
        exit_time: TimeOfDay::from_minutes(14 * 60 + 59),
        dte: crate::risk_engine::strategy::DteSelection::default(),
        entry_condition: EntryCondition::Always,
        legs: vec![LegDefinition {
            side: Side::Call,
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
        }],
        overall: OverallRisk::default(),
        loss_coverage: LossCoverage::Breakeven,
    }
}

#[test]
fn saving_then_loading_round_trips_every_field() {
    let _sandbox = Sandbox::new();
    let original = strategy("nifty-atm", "NIFTY");
    save(&original).expect("save");

    let loaded = load("nifty-atm").expect("load");
    assert_eq!(loaded.id, original.id);
    assert_eq!(loaded.underlying, "NIFTY");
    assert_eq!(loaded.entry_time.minutes(), 556);
    assert_eq!(loaded.exit_time.minutes(), 899);
    assert_eq!(loaded.loss_coverage, LossCoverage::Breakeven);
    assert_eq!(loaded.legs.len(), 1);
    assert_eq!(loaded.legs[0].stop_loss.unwrap().value, 75.0);
}

#[test]
fn an_invalid_strategy_is_refused_before_it_reaches_disk() {
    let _sandbox = Sandbox::new();
    let mut broken = strategy("../escape", "NIFTY");
    broken.legs.clear();

    assert!(save(&broken).is_err());
    assert!(load("../escape").is_err(), "nothing was written");
}

#[test]
fn listing_returns_saved_strategies_sorted_and_skips_the_active_marker() {
    let _sandbox = Sandbox::new();
    save(&strategy("zeta", "SENSEX")).expect("save zeta");
    save(&strategy("alpha", "NIFTY")).expect("save alpha");
    activate("alpha").expect("activate");

    let ids: Vec<String> = list().strategies.into_iter().map(|item| item.id).collect();
    assert_eq!(ids, vec!["alpha".to_owned(), "zeta".to_owned()]);
}

#[test]
fn activation_is_a_set_and_only_accepts_saved_strategies() {
    let _sandbox = Sandbox::new();
    save(&strategy("one", "NIFTY")).expect("save");
    save(&strategy("two", "BANKNIFTY")).expect("save");

    assert!(active().is_empty());
    assert!(
        activate("ghost").is_err(),
        "an unsaved strategy cannot be activated"
    );

    activate("one").expect("activate one");
    activate("two").expect("activate two");
    activate("one").expect("activating twice is idempotent");
    assert_eq!(active().len(), 2);

    deactivate("one").expect("deactivate");
    assert_eq!(
        active().into_iter().collect::<Vec<_>>(),
        vec!["two".to_owned()]
    );
}

#[test]
fn removing_a_strategy_also_drops_it_from_the_active_set() {
    let _sandbox = Sandbox::new();
    save(&strategy("doomed", "NIFTY")).expect("save");
    activate("doomed").expect("activate");
    assert_eq!(active().len(), 1);

    remove("doomed").expect("remove");
    assert!(load("doomed").is_err());
    assert!(
        active().is_empty(),
        "a deleted strategy must not stay activated"
    );
}

#[test]
fn several_strategies_can_target_different_indices_at_once() {
    let _sandbox = Sandbox::new();
    for (id, underlying) in [
        ("n", "NIFTY"),
        ("b", "BANKNIFTY"),
        ("s", "SENSEX"),
        ("f", "FINNIFTY"),
    ] {
        save(&strategy(id, underlying)).expect("save");
        activate(id).expect("activate");
    }

    assert_eq!(active().len(), 4);
    let underlyings: Vec<String> = list()
        .strategies
        .into_iter()
        .map(|item| item.underlying)
        .collect();
    assert_eq!(
        underlyings,
        vec![
            "BANKNIFTY".to_owned(),
            "FINNIFTY".to_owned(),
            "NIFTY".to_owned(),
            "SENSEX".to_owned()
        ]
    );
}

#[test]
fn concurrent_activation_of_different_strategies_loses_nobody() {
    let _sandbox = Sandbox::new();
    let ids: Vec<String> = (0..12).map(|n| format!("s{n}")).collect();
    for id in &ids {
        save(&strategy(id, "NIFTY")).expect("save");
    }

    std::thread::scope(|scope| {
        for id in &ids {
            scope.spawn(move || activate(id).expect("activate"));
        }
    });

    let active = active();
    assert_eq!(
        active.len(),
        ids.len(),
        "every activation must survive: got {active:?}"
    );
}

#[test]
fn concurrent_writes_leave_one_coherent_file_and_no_temp_droppings() {
    let _sandbox = Sandbox::new();
    save(&strategy("target", "NIFTY")).expect("save");

    std::thread::scope(|scope| {
        for turn in 0..16 {
            scope.spawn(move || {
                if turn % 2 == 0 {
                    activate("target").expect("concurrent activation must succeed");
                } else {
                    deactivate("target").expect("concurrent deactivation must succeed");
                }
            });
        }
    });

    let from_reader = active();
    let path = crate::config::data_path("data/state/active.json");
    let body = std::fs::read_to_string(&path).expect("activation state file must exist");
    let on_disk: std::collections::BTreeSet<String> =
        serde_json::from_str(&body).expect("activation state file must contain valid JSON");
    assert_eq!(
        from_reader, on_disk,
        "the active set and its file must not disagree"
    );

    for directory in ["data/strategies", "data/state"] {
        let strays = std::fs::read_dir(crate::config::data_path(directory))
            .expect("store directory must exist")
            .map(|entry| entry.expect("store directory entry must be readable"))
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp"))
            .count();
        assert_eq!(strays, 0, "no temp files may be left behind in {directory}");
    }
}

#[test]
fn a_strategy_named_active_cannot_overwrite_the_activation_set() {
    let sandbox = Sandbox::new();

    save(&strategy("keeper", "NIFTY")).expect("save keeper");
    activate("keeper").expect("activate keeper");
    assert!(active().contains("keeper"));

    save(&strategy("active", "BANKNIFTY")).expect("a strategy may legitimately be called active");
    activate("active").expect("activate active");

    let armed = active();
    assert!(
        armed.contains("keeper") && armed.contains("active"),
        "saving a strategy called active must not have clobbered the activation set: {armed:?}"
    );

    let reloaded = load("active").expect("the strategy called active is still a strategy");
    assert_eq!(reloaded.underlying, "BANKNIFTY");

    let definitions = sandbox.path().join("data/strategies");
    let state = sandbox.path().join("data/state");
    assert!(
        definitions.join("active.json").exists(),
        "the strategy lives under the definitions directory"
    );
    assert!(
        state.join("active.json").exists(),
        "the activation set lives under the state directory"
    );
}

#[test]
fn an_id_that_would_escape_the_store_is_refused_by_every_operation() {
    let _sandbox = Sandbox::new();

    save(&strategy("victim", "NIFTY")).expect("save");

    let escapes = [
        "../victim",
        "../../victim",
        "..",
        ".",
        "a/b",
        "a\\b",
        "/etc/passwd",
        "C:\\windows",
        "with space",
        "dot.dot",
        "new\nline",
        "null\0byte",
        "",
    ];

    for bad in escapes {
        assert!(
            load(bad).is_err(),
            "load must refuse {bad:?} rather than reading an arbitrary path"
        );
        assert!(
            remove(bad).is_err(),
            "remove must refuse {bad:?} rather than unlinking an arbitrary path"
        );
        assert!(activate(bad).is_err(), "activate must refuse {bad:?}");
        assert!(deactivate(bad).is_err(), "deactivate must refuse {bad:?}");

        let mut forged = strategy("placeholder", "NIFTY");
        forged.id = bad.to_owned();
        assert!(save(&forged).is_err(), "save must refuse {bad:?}");
    }

    assert!(
        load("victim").is_ok(),
        "the refusals must not have damaged a real strategy"
    );
}

#[test]
fn an_over_long_id_is_refused_before_it_reaches_the_filesystem() {
    let _sandbox = Sandbox::new();

    let long = "a".repeat(crate::risk_engine::strategy::MAX_ID_BYTES + 1);
    let mut oversized = strategy("placeholder", "NIFTY");
    oversized.id = long.clone();
    assert!(save(&oversized).is_err());
    assert!(load(&long).is_err());

    let at_limit = "b".repeat(crate::risk_engine::strategy::MAX_ID_BYTES);
    let mut allowed = strategy("placeholder", "NIFTY");
    allowed.id = at_limit.clone();
    assert!(
        save(&allowed).is_ok(),
        "the limit itself is still a usable id"
    );
    assert!(load(&at_limit).is_ok());
}

#[test]
fn the_activation_set_migrates_out_of_the_strategy_directory_once_and_idempotently() {
    let sandbox = Sandbox::new();
    let root = sandbox.path();
    let definitions = root.join("data/strategies");
    let state = root.join("data/state");

    std::fs::create_dir_all(&definitions).expect("mkdir");
    std::fs::write(
        definitions.join("active.json"),
        "[\n  \"legacy-one\",\n  \"legacy-two\"\n]",
    )
    .expect("seed the legacy activation file");

    migrate().expect("first migration");

    assert!(
        !definitions.join("active.json").exists(),
        "the legacy file is moved, not copied"
    );
    assert!(state.join("active.json").exists());
    let armed = active();
    assert!(
        armed.contains("legacy-one") && armed.contains("legacy-two"),
        "the activation set survived the move: {armed:?}"
    );

    migrate().expect("second migration");
    let again = active();
    assert_eq!(armed, again, "migration is idempotent");
}

#[test]
fn migration_leaves_a_real_strategy_called_active_alone() {
    let sandbox = Sandbox::new();

    save(&strategy("active", "SENSEX")).expect("save a strategy called active");
    migrate().expect("migrate");

    let definitions = sandbox.path().join("data/strategies");
    assert!(
        definitions.join("active.json").exists(),
        "a strategy file must not be mistaken for the activation set and moved"
    );
    let kept = load("active").expect("still loadable");
    assert_eq!(kept.underlying, "SENSEX");
    assert!(
        active().is_empty(),
        "no activation set existed, so none was invented"
    );
}

#[test]
fn existing_valid_strategies_remain_readable_after_the_layout_change() {
    let sandbox = Sandbox::new();
    let definitions = sandbox.path().join("data/strategies");
    std::fs::create_dir_all(&definitions).expect("mkdir");

    let existing = strategy("pre-existing", "FINNIFTY");
    let body = serde_json::to_string_pretty(&existing).expect("encode");
    std::fs::write(definitions.join("pre-existing.json"), body).expect("seed");

    migrate().expect("migrate");

    let loaded = load("pre-existing").expect("a strategy written by the old layout still loads");
    assert_eq!(loaded.underlying, "FINNIFTY");
    assert!(
        list()
            .strategies
            .iter()
            .any(|item| item.id == "pre-existing"),
        "and it still lists"
    );
}
