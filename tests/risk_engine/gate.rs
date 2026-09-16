use super::expiry_gate_met;

#[test]
fn zero_and_one_dte_arm_when_the_limit_is_one() {
    assert!(expiry_gate_met(Some(0), Some(1)), "0DTE is expiry day itself");
    assert!(expiry_gate_met(Some(1), Some(1)), "1DTE is the day before");
}

#[test]
fn a_further_expiry_does_not_arm() {
    for days in [2, 3, 6, 30] {
        assert!(
            !expiry_gate_met(Some(days), Some(1)),
            "{days} days out must not arm a 0-1 DTE strategy"
        );
    }
}

#[test]
fn an_expiry_already_past_never_arms() {
    for days in [-1, -6, -365] {
        assert!(
            !expiry_gate_met(Some(days), Some(1)),
            "a past expiry must not arm even inside the limit"
        );
        assert!(
            !expiry_gate_met(Some(days), None),
            "a past expiry must not arm when no limit is set either"
        );
    }
}

#[test]
fn no_limit_means_any_live_expiry() {
    assert!(expiry_gate_met(Some(0), None));
    assert!(expiry_gate_met(Some(45), None));
}

#[test]
fn an_unreadable_expiry_blocks_rather_than_passes() {
    assert!(
        !expiry_gate_met(None, Some(1)),
        "not knowing the expiry must fail closed, never open"
    );
    assert!(!expiry_gate_met(None, None));
}

#[test]
fn a_zero_limit_arms_only_on_expiry_day() {
    assert!(expiry_gate_met(Some(0), Some(0)));
    assert!(!expiry_gate_met(Some(1), Some(0)));
}
