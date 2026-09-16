use serde::{Deserialize, Serialize};

use crate::dhan_api::instruments::{from_strike_units, to_strike_units};
use crate::option_chain::metrics::{market_atm_index, spot_atm};
use crate::option_chain::table::{Block, OptionTable};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Side {
    Call,
    Put,
}

impl Side {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Call => "CE",
            Self::Put => "PE",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Moneyness {
    Atm,
    Itm,
    Otm,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum StrikeCriteria {
    Relative {
        moneyness: Moneyness,
        #[serde(default)]
        steps: u32,
    },
    ClosestPremium {
        target: f64,
    },
    PremiumRange {
        lower: f64,
        upper: f64,
    },
    PremiumAtLeast {
        threshold: f64,
    },
    PremiumAtMost {
        threshold: f64,
    },
    StraddleWidth {
        multiplier: f64,
        away_from_atm: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct ResolvedStrike {
    pub(crate) side: Side,
    pub(crate) strike: f64,
    pub(crate) row: usize,
    pub(crate) security_id: String,
    pub(crate) ltp: f64,
    pub(crate) bid: f64,
    pub(crate) ask: f64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub(crate) enum StrikeError {
    NoSpotPrice,
    EmptyChain,
    UnknownStrikeStep,
    StrikeOutsideChain { wanted: String },
    NoStrikeMeetsCriteria,
    NoContractAtStrike { strike: String, side: &'static str },
}

fn block_of(table: &OptionTable, side: Side) -> &Block {
    match side {
        Side::Call => &table.calls,
        Side::Put => &table.puts,
    }
}

fn tradable_premium(table: &OptionTable, side: Side, row: usize) -> Option<f64> {
    let block = block_of(table, side);
    if block.security_id.get(row)?.is_none() {
        return None;
    }
    let ltp = *block.ltp.get(row)?;
    (ltp.is_finite() && ltp > 0.0).then_some(ltp)
}

fn resolve_relative(
    table: &OptionTable,
    side: Side,
    moneyness: Moneyness,
    steps: u32,
) -> Result<usize, StrikeError> {
    let anchor = spot_atm(table).ok_or(StrikeError::NoSpotPrice)?;
    if table.strike_step_units <= 0 {
        return Err(StrikeError::UnknownStrikeStep);
    }

    let offset = i64::from(steps) * table.strike_step_units;
    let direction = match (moneyness, side) {
        (Moneyness::Atm, _) => 0,
        (Moneyness::Itm, Side::Call) | (Moneyness::Otm, Side::Put) => -1,
        (Moneyness::Itm, Side::Put) | (Moneyness::Otm, Side::Call) => 1,
    };
    let wanted = to_strike_units(anchor) + direction * offset;

    table
        .strike_units
        .binary_search(&wanted)
        .map_err(|_| StrikeError::StrikeOutsideChain {
            wanted: format!("{:.2}", from_strike_units(wanted)),
        })
}

fn best_by<F>(table: &OptionTable, side: Side, score: F) -> Option<usize>
where
    F: Fn(f64) -> Option<f64>,
{
    let mut best: Option<(usize, f64)> = None;
    for row in 0..table.len() {
        let Some(premium) = tradable_premium(table, side, row) else {
            continue;
        };
        let Some(value) = score(premium) else {
            continue;
        };
        if best.is_none_or(|(_, current)| value < current) {
            best = Some((row, value));
        }
    }
    best.map(|(row, _)| row)
}


pub(crate) fn resolve(
    table: &OptionTable,
    side: Side,
    criteria: &StrikeCriteria,
) -> Result<ResolvedStrike, StrikeError> {
    if table.strikes.is_empty() {
        return Err(StrikeError::EmptyChain);
    }

    let row = match criteria {
        StrikeCriteria::Relative { moneyness, steps } => {
            resolve_relative(table, side, *moneyness, *steps)?
        }
        StrikeCriteria::ClosestPremium { target } => {
            best_by(table, side, |premium| Some((premium - target).abs()))
                .ok_or(StrikeError::NoStrikeMeetsCriteria)?
        }
        StrikeCriteria::PremiumRange { lower, upper } => best_by(table, side, |premium| {
            (premium >= *lower && premium <= *upper).then_some(0.0)
        })
        .ok_or(StrikeError::NoStrikeMeetsCriteria)?,
        StrikeCriteria::PremiumAtLeast { threshold } => best_by(table, side, |premium| {
            (premium >= *threshold).then(|| premium - threshold)
        })
        .ok_or(StrikeError::NoStrikeMeetsCriteria)?,
        StrikeCriteria::PremiumAtMost { threshold } => best_by(table, side, |premium| {
            (premium <= *threshold).then(|| threshold - premium)
        })
        .ok_or(StrikeError::NoStrikeMeetsCriteria)?,
        StrikeCriteria::StraddleWidth {
            multiplier,
            away_from_atm,
        } => {
            let anchor = market_atm_index(table).ok_or(StrikeError::NoSpotPrice)?;
            let straddle = table.calls.ltp[anchor] + table.puts.ltp[anchor];
            if !(straddle.is_finite() && straddle > 0.0) {
                return Err(StrikeError::NoStrikeMeetsCriteria);
            }
            if table.strike_step_units <= 0 {
                return Err(StrikeError::UnknownStrikeStep);
            }
            let step = from_strike_units(table.strike_step_units);
            let offset = ((straddle * multiplier) / step).round() * step;
            let signed = match (away_from_atm, side) {
                (true, Side::Call) | (false, Side::Put) => offset,
                (true, Side::Put) | (false, Side::Call) => -offset,
            };
            let wanted = to_strike_units(table.strikes[anchor] + signed);
            table
                .find_nearest_strike_index(wanted)
                .ok_or(StrikeError::EmptyChain)?
        }
    };

    let block = block_of(table, side);
    let security_id = block
        .security_id
        .get(row)
        .and_then(|id| id.clone())
        .ok_or_else(|| StrikeError::NoContractAtStrike {
            strike: format!("{:.2}", table.strikes[row]),
            side: side.label(),
        })?;

    Ok(ResolvedStrike {
        side,
        strike: table.strikes[row],
        row,
        security_id,
        ltp: block.ltp[row],
        bid: block.bid[row],
        ask: block.ask[row],
    })
}

#[cfg(test)]
#[path = "../../tests/risk_engine/strike.rs"]
mod tests;
