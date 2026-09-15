use std::collections::BTreeMap;

use super::master::{
    ChainKind, InstrumentMaster, OptionContract, OptionType, UnderlyingKey, from_strike_units,
};

pub(crate) const MAX_STRIKES_EACH_SIDE: usize = 10;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OptionChain {
    pub(crate) underlying: UnderlyingKey,
    pub(crate) kind: ChainKind,
    pub(crate) expiry: String,
    pub(crate) lot_size: u32,
    pub(crate) strike_step: i64,
    pub(crate) strikes: Vec<i64>,
    calls: BTreeMap<i64, OptionContract>,
    puts: BTreeMap<i64, OptionContract>,
}

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
    pub(crate) fn legs(&self, strikes: &[i64]) -> Vec<&OptionContract> {
        strikes
            .iter()
            .flat_map(|strike| [self.calls.get(strike), self.puts.get(strike)])
            .flatten()
            .collect()
    }

    pub(crate) fn leg_count(&self, strikes: &[i64]) -> usize {
        self.legs(strikes).len()
    }
}

pub(crate) fn index_option_chains(
    master: &InstrumentMaster,
    as_of: &str,
) -> BTreeMap<UnderlyingKey, OptionChain> {
    let mut grouped: BTreeMap<UnderlyingKey, BTreeMap<&str, Vec<&OptionContract>>> =
        BTreeMap::new();

    for contract in &master.options {
        if contract.expiry.as_str() < as_of {
            continue;
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

pub(crate) fn strike_step(strikes: &[i64]) -> i64 {
    if strikes.len() < 2 {
        return 0;
    }
    let mut gaps: Vec<i64> = strikes.windows(2).map(|pair| pair[1] - pair[0]).collect();
    gaps.sort_unstable();
    gaps[gaps.len() / 2]
}

pub(crate) fn atm_strike(strikes: &[i64], spot_units: i64) -> Option<i64> {
    strikes
        .iter()
        .copied()
        .min_by_key(|strike| (strike.abs_diff(spot_units), *strike))
}

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
