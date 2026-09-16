use super::*;

#[test]
fn days_between_counts_forward_and_backward() {
    assert_eq!(days_between("2026-09-16", "2026-09-22").unwrap(), 6);
    assert_eq!(days_between("2026-09-16", "2026-09-17").unwrap(), 1);
    assert_eq!(days_between("2026-09-16", "2026-09-16").unwrap(), 0);
    assert_eq!(
        days_between("2026-09-22", "2026-09-16").unwrap(),
        -6,
        "an expiry already past reads negative rather than wrapping"
    );
}

#[test]
fn days_between_crosses_month_and_year_boundaries() {
    assert_eq!(days_between("2026-02-28", "2026-03-01").unwrap(), 1);
    assert_eq!(days_between("2026-12-31", "2027-01-01").unwrap(), 1);
    assert_eq!(days_between("2026-01-31", "2026-02-01").unwrap(), 1);
}

#[test]
fn days_between_respects_leap_years() {
    assert_eq!(
        days_between("2024-02-28", "2024-03-01").unwrap(),
        2,
        "2024 has a 29th"
    );
    assert_eq!(
        days_between("2026-02-28", "2026-03-01").unwrap(),
        1,
        "2026 does not"
    );
    assert_eq!(days_between("2024-01-01", "2025-01-01").unwrap(), 366);
    assert_eq!(days_between("2026-01-01", "2027-01-01").unwrap(), 365);
}

#[test]
fn days_between_round_trips_against_the_days_to_date_inverse() {
    for days in [0_i64, 1, 19_000, 20_712, 25_000, -1, -3_650] {
        let (year, month, day) = civil_from_days(days);
        let iso = format!("{year:04}-{month:02}-{day:02}");
        assert_eq!(
            days_between("1970-01-01", &iso).unwrap(),
            days,
            "{iso} must convert back to {days}"
        );
    }
}

#[test]
fn a_malformed_date_is_an_error_not_a_silent_zero() {
    assert!(days_between("2026-13-01", "2026-09-16").is_err());
    assert!(days_between("2026-09-16", "2026-02-30").is_err());
    assert!(days_between("not-a-date", "2026-09-16").is_err());
    assert!(days_between("2026-09-16", "20260917").is_err());
}


#[test]
fn shifting_a_date_agrees_with_the_span_it_was_asked_for() {
    for days in [0_i64, 1, 6, 7, 30, 365, -1, -6, -400] {
        for from in [
            "2026-09-16",
            "2026-12-31",
            "2026-01-01",
            "2024-02-28",
            "2026-02-28",
            "2026-03-01",
        ] {
            let shifted = shift_iso_date(from, days).expect("shift");
            assert_eq!(
                days_between(from, &shifted).expect("span"),
                days,
                "{from} shifted by {days} gave {shifted}"
            );
        }
    }
}

#[test]
fn shifting_lands_on_the_expected_calendar_day() {
    assert_eq!(shift_iso_date("2026-09-16", 6).unwrap(), "2026-09-22");
    assert_eq!(shift_iso_date("2026-09-16", 0).unwrap(), "2026-09-16");
    assert_eq!(shift_iso_date("2026-12-31", 1).unwrap(), "2027-01-01");
    assert_eq!(shift_iso_date("2026-02-28", 1).unwrap(), "2026-03-01");
    assert_eq!(
        shift_iso_date("2024-02-28", 1).unwrap(),
        "2024-02-29",
        "2024 has a 29th"
    );
    assert_eq!(shift_iso_date("2027-01-01", -1).unwrap(), "2026-12-31");
}

#[test]
fn shifting_a_malformed_date_is_an_error() {
    assert!(shift_iso_date("2026-13-01", 1).is_err());
    assert!(shift_iso_date("20260916", 1).is_err());
}
