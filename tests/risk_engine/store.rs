use super::*;
use crate::risk_engine::strategy::{
    Action, EntryCondition, LegDefinition, LossCoverage, OverallRisk, RiskMethod, Threshold,
    TimeOfDay,
};
use crate::risk_engine::strike::{Moneyness, Side, StrikeCriteria};

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

static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Sandbox {
    _guard: std::sync::MutexGuard<'static, ()>,
    _directory: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let guard = HOME_LOCK.lock().unwrap_or_else(|error| {
            HOME_LOCK.clear_poison();
            error.into_inner()
        });
        let directory = tempfile::tempdir().expect("temp dir");
        unsafe {
            std::env::set_var("BLACKBOX_HOME", directory.path());
        }
        Self {
            _guard: guard,
            _directory: directory,
        }
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
    let underlyings: Vec<String> =
        list().strategies.into_iter().map(|item| item.underlying).collect();
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

    // Activating is read, insert, write. Run them together: without a lock one
    // thread's set overwrites another's and activations silently vanish.
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
                    let _ = activate("target");
                } else {
                    let _ = deactivate("target");
                }
            });
        }
    });

    // Whatever the interleaving decided, the file must be readable and agree with
    // what the reader reports - a shared temp path used to publish another
    // writer's bytes and leave the set and the file disagreeing.
    let from_reader = active();
    let path = crate::config::data_path("data/strategies/active.json");
    let on_disk: std::collections::BTreeSet<String> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|body| serde_json::from_str(&body).ok())
        .unwrap_or_default();
    assert_eq!(
        from_reader, on_disk,
        "the active set and its file must not disagree"
    );

    let strays = std::fs::read_dir(crate::config::data_path("data/strategies"))
        .expect("dir")
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp"))
        .count();
    assert_eq!(strays, 0, "no temp files may be left behind");
}
