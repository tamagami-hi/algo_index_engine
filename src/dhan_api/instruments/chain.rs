//! Turning flat master rows into per-underlying option chains, and picking the
//! strike window around the money.
//!
//! Two things happen here, and they are deliberately separate because they need
//! different inputs:
//!
//! 1. [`index_option_chains`] needs only the master and today's date. It can run
//!    before any connection exists.
//! 2. [`select_window`] needs a live spot price, which only the feed can supply.
//!
//! That split is what makes the startup order work: the chains and the spot
//! subscription list are built up front, and the option legs are chosen once the
//! first spot ticks arrive.

use std::collections::BTreeMap;

use super::master::{
    ChainKind, InstrumentMaster, OptionContract, OptionType, UnderlyingKey, from_strike_units,
};

/// The widest window this module will build, in strikes each side of ATM.
///
/// A cap exists because the window size multiplies straight into the subscription
/// count: every extra strike is two more instruments per underlying, across ~430
/// underlyings.
pub(crate) const MAX_STRIKES_EACH_SIDE: usize = 10;

/// One underlying's option chain for a single expiry.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OptionChain {
    pub(crate) underlying: UnderlyingKey,
    pub(crate) kind: ChainKind,
    /// The nearest expiry that has not passed, `YYYY-MM-DD`.
    pub(crate) expiry: String,
    pub(crate) lot_size: u32,
    /// Typical gap between adjacent strikes, in strike units. Used for the
    /// re-centring band; 0 when the chain has fewer than two strikes.
    pub(crate) strike_step: i64,
    /// Strikes that have BOTH a call and a put, ascending.
    pub(crate) strikes: Vec<i64>,
    calls: BTreeMap<i64, OptionContract>,
    puts: BTreeMap<i64, OptionContract>,
}

/// A chosen window of strikes around the money.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StrikeWindow {
    pub(crate) atm_strike_units: i64,
    pub(crate) strikes: Vec<i64>,
}

impl StrikeWindow {
    pub(crate) fn atm_strike(&self) -> f64 {
        from_strike_units(self.atm_strike_units)
    }
}

impl OptionChain {
    /// The CE and PE contracts for the given strikes, calls first at each strike.
    pub(crate) fn legs(&self, strikes: &[i64]) -> Vec<&OptionContract> {
        strikes
            .iter()
            .flat_map(|strike| [self.calls.get(strike), self.puts.get(strike)])
            .flatten()
            .collect()
    }

    /// Number of contracts a window of `strikes` would subscribe.
    pub(crate) fn leg_count(&self, strikes: &[i64]) -> usize {
        self.legs(strikes).len()
    }
}

/// Group every option contract into per-underlying chains, keeping only the nearest
/// expiry that has not passed.
///
/// `as_of` is an IST `YYYY-MM-DD`. An expiry equal to `as_of` is kept: a contract
/// expiring today still trades until the close.
pub(crate) fn index_option_chains(
    master: &InstrumentMaster,
    as_of: &str,
) -> BTreeMap<UnderlyingKey, OptionChain> {
    // underlying -> expiry -> contracts
    let mut grouped: BTreeMap<UnderlyingKey, BTreeMap<&str, Vec<&OptionContract>>> =
        BTreeMap::new();

    for contract in &master.options {
        if contract.expiry.as_str() < as_of {
            continue; // already expired
        }
        let key = UnderlyingKey::new(contract.segment, contract.underlying_symbol.as_str());
        grouped
            .entry(key)
            .or_default()
            .entry(contract.expiry.as_str())
            .or_default()
            .push(contract);
    }

    let mut chains = BTreeMap::new();
    for (key, by_expiry) in grouped {
        // A BTreeMap over ISO date strings is already in chronological order, so the
        // first entry is the nearest live expiry.
        let Some((expiry, contracts)) = by_expiry.into_iter().next() else {
            continue;
        };

        let mut calls = BTreeMap::new();
        let mut puts = BTreeMap::new();
        let mut lot_size = 0;
        let mut kind = ChainKind::Stock;
        for contract in contracts {
            match contract.option_type {
                OptionType::Call => calls.insert(contract.strike_units, contract.clone()),
                OptionType::Put => puts.insert(contract.strike_units, contract.clone()),
            };
            if lot_size == 0 && contract.lot_size > 0 {
                lot_size = contract.lot_size;
            }
            kind = contract.kind;
        }

        // Only strikes with both legs are usable: a chain view with a call but no put
        // at the same strike is not something to act on, and a lone leg would waste a
        // subscription slot.
        let strikes: Vec<i64> = calls
            .keys()
            .copied()
            .filter(|strike| puts.contains_key(strike))
            .collect();
        if strikes.is_empty() {
            continue;
        }

        chains.insert(
            key.clone(),
            OptionChain {
                underlying: key,
                kind,
                expiry: expiry.to_owned(),
                lot_size,
                strike_step: strike_step(&strikes),
                strikes,
                calls,
                puts,
            },
        );
    }
    chains
}

/// The typical gap between adjacent strikes, as the MEDIAN of the gaps.
///
/// Median rather than mean because real chains have holes: a delisted or newly added
/// strike leaves a double-width gap, and one such outlier would drag a mean far off
/// the true step. The step feeds the re-centring band, so being wrong here makes the
/// window either thrash or go stale.
pub(crate) fn strike_step(strikes: &[i64]) -> i64 {
    if strikes.len() < 2 {
        return 0;
    }
    let mut gaps: Vec<i64> = strikes.windows(2).map(|pair| pair[1] - pair[0]).collect();
    gaps.sort_unstable();
    gaps[gaps.len() / 2]
}

/// The strike closest to `spot_units`.
///
/// Ties go to the lower strike, which is arbitrary but stable — the same spot always
/// picks the same ATM, so the window does not flip between two strikes.
pub(crate) fn atm_strike(strikes: &[i64], spot_units: i64) -> Option<i64> {
    strikes
        .iter()
        .copied()
        .min_by_key(|strike| (strike.abs_diff(spot_units), *strike))
}

/// Pick ATM and `each_side` strikes either side of it.
///
/// The window is clamped to the ends of the chain rather than padded, so an underlying
/// trading near the edge of its listed strikes yields a short window instead of none.
pub(crate) fn select_window(
    strikes: &[i64],
    spot_units: i64,
    each_side: usize,
) -> Option<StrikeWindow> {
    let each_side = each_side.min(MAX_STRIKES_EACH_SIDE);
    let atm = atm_strike(strikes, spot_units)?;
    let centre = strikes.iter().position(|strike| *strike == atm)?;

    let start = centre.saturating_sub(each_side);
    let end = (centre + each_side + 1).min(strikes.len());

    Some(StrikeWindow {
        atm_strike_units: atm,
        strikes: strikes[start..end].to_vec(),
    })
}

/// Whether a window centred on `current_atm_units` should be rebuilt for a new spot.
///
/// The band is half a strike step plus a `hysteresis` fraction of a step. Without the
/// extra fraction, a spot sitting exactly between two strikes would re-centre on
/// every tick, and each re-centre means unsubscribing and resubscribing option legs.
pub(crate) fn should_recentre(
    current_atm_units: i64,
    spot_units: i64,
    strike_step: i64,
    hysteresis: f64,
) -> bool {
    if strike_step <= 0 {
        return false;
    }
    let drift = current_atm_units.abs_diff(spot_units) as f64;
    drift > strike_step as f64 * (0.5 + hysteresis.max(0.0))
}
