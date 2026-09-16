use super::*;

fn raw(premium: f64, bid: f64, ask: f64) -> RawQuote {
    RawQuote {
        quoted: true,
        received_at: monotonic_millis(),
        premium,
        bid,
        ask,
    }
}

#[test]
fn a_price_is_usable_only_when_positive_and_finite() {
    assert_eq!(usable_price(129.3), Some(129.3));
    assert_eq!(usable_price(0.05), Some(0.05));

    for unusable in [
        0.0,
        -0.01,
        -100.0,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ] {
        assert_eq!(
            usable_price(unusable),
            None,
            "{unusable} is not a tradeable price"
        );
    }
}

#[test]
fn a_contract_that_never_quoted_is_never_usable() {
    let never = RawQuote {
        quoted: false,
        received_at: monotonic_millis(),
        premium: 100.0,
        bid: 99.0,
        ask: 101.0,
    };
    assert_eq!(
        inspect_quote(never, None),
        Err(QuoteProblem::NeverQuoted),
        "prices carried by an unquoted row are leftovers, not a market"
    );
}

#[test]
fn a_quote_older_than_the_limit_is_stale() {
    let stamped = RawQuote {
        quoted: true,
        received_at: 1_000,
        premium: 100.0,
        bid: 99.0,
        ask: 101.0,
    };

    assert_eq!(
        inspect_quote_at(stamped, None, 11_001, 10_000),
        Err(QuoteProblem::Stale {
            age_ms: 10_001,
            limit_ms: 10_000,
        }),
        "one millisecond past the limit is stale"
    );

    let exactly_at_limit = inspect_quote_at(stamped, None, 11_000, 10_000);
    assert!(
        exactly_at_limit.is_ok(),
        "the limit itself is still fresh, {exactly_at_limit:?}"
    );

    let inside = inspect_quote_at(stamped, None, 5_000, 10_000).expect("fresh");
    assert_eq!(inside.age_ms, 4_000);
}

#[test]
fn staleness_never_reads_negative_when_a_stamp_is_ahead_of_now() {
    let ahead = RawQuote {
        quoted: true,
        received_at: 50_000,
        premium: 100.0,
        bid: 99.0,
        ask: 101.0,
    };
    let quote = inspect_quote_at(ahead, None, 1_000, 10_000).expect("must not wrap into staleness");
    assert_eq!(
        quote.age_ms, 0,
        "a stamp ahead of now reads as brand new rather than wrapping to a huge age"
    );
}

#[test]
fn a_quote_inside_the_limit_is_fresh() {
    let quote = inspect_quote(raw(100.0, 99.0, 101.0), None).expect("fresh");
    assert_eq!(quote.premium, 100.0);
    assert!(
        quote.age_ms <= freshness().option_max_age_ms,
        "a quote taken now cannot be stale"
    );
}

#[test]
fn a_missing_premium_is_reported_rather_than_treated_as_zero() {
    for premium in [0.0, -1.0, f64::NAN] {
        assert_eq!(
            inspect_quote(raw(premium, 99.0, 101.0), None),
            Err(QuoteProblem::PremiumUnavailable),
            "{premium} is not a premium"
        );
    }
}

#[test]
fn the_side_a_trade_needs_must_be_present() {
    let no_bid = inspect_quote(raw(100.0, 0.0, 101.0), Some(QuoteSide::Bid));
    assert_eq!(
        no_bid,
        Err(QuoteProblem::NoUsableSide {
            side: QuoteSide::Bid
        }),
        "a leg that must sell needs a bid to sell into"
    );

    let no_ask = inspect_quote(raw(100.0, 99.0, 0.0), Some(QuoteSide::Ask));
    assert_eq!(
        no_ask,
        Err(QuoteProblem::NoUsableSide {
            side: QuoteSide::Ask
        }),
        "a leg that must buy needs an ask to buy from"
    );

    assert!(
        inspect_quote(raw(100.0, 0.0, 101.0), Some(QuoteSide::Ask)).is_ok(),
        "a missing bid does not block a leg that only needs the ask"
    );
    assert!(
        inspect_quote(raw(100.0, 0.0, 0.0), None).is_ok(),
        "with no side required, a premium alone is usable"
    );
}

#[test]
fn a_usable_quote_reports_only_the_sides_that_are_real() {
    let one_sided = inspect_quote(raw(100.0, 99.5, 0.0), None).expect("usable");
    assert_eq!(one_sided.bid, Some(99.5));
    assert_eq!(one_sided.ask, None, "an absent ask is absent, not zero");
    assert_eq!(one_sided.side(QuoteSide::Bid), Some(99.5));
    assert_eq!(one_sided.side(QuoteSide::Ask), None);
}

#[test]
fn the_monotonic_clock_only_moves_forward() {
    let first = monotonic_millis();
    let second = monotonic_millis();
    assert!(
        second >= first,
        "age must not depend on a wall clock that can be adjusted backwards"
    );
    assert_eq!(
        age_since(second.saturating_add(10_000)),
        0,
        "a stamp from the future reads as brand new rather than wrapping"
    );
}

#[test]
fn the_freshness_limits_are_defined_in_one_place() {
    let limits = freshness();
    assert!(limits.underlying_max_age_ms > 0);
    assert!(limits.option_max_age_ms > 0);
    assert!(limits.feed_silence_max_ms > 0);
    assert!(
        limits.option_max_age_ms >= limits.underlying_max_age_ms,
        "an index ticks continuously; a single strike may not, so it gets more room"
    );
}
