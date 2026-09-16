pub(crate) mod store;
pub(crate) mod strategy;
pub(crate) mod strike;

use serde::Serialize;

use crate::dhan_api::instruments::{days_between, ist_minutes_now, ist_today};
use crate::option_chain::table::OptionTable;
use strategy::Strategy;
use strike::{ResolvedStrike, StrikeError, resolve};

#[cfg(test)]
#[path = "../../tests/risk_engine/gate.rs"]
mod tests;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ResolvedLeg {
    pub(crate) leg: usize,
    #[serde(flatten)]
    pub(crate) strike: ResolvedStrike,
    pub(crate) action: strategy::Action,
    pub(crate) lots: u32,
    pub(crate) quantity: u32,
    pub(crate) stop_price: Option<f64>,
    pub(crate) target_price: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct SizingProblem {
    pub(crate) problem: &'static str,
    pub(crate) underlying: String,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct LegProblem {
    pub(crate) leg: usize,
    pub(crate) side: &'static str,
    #[serde(flatten)]
    pub(crate) error: StrikeError,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Resolution {
    pub(crate) id: String,
    pub(crate) underlying: String,
    pub(crate) expiry: String,
    pub(crate) spot_price: f64,
    pub(crate) spot_atm: Option<f64>,
    pub(crate) entry_condition_met: bool,
    pub(crate) entry_window_open: bool,
    pub(crate) entry_closes_at: String,
    pub(crate) before_hard_exit: bool,
    pub(crate) minutes_until_exit: i64,
    pub(crate) days_to_expiry: Option<i64>,
    pub(crate) expiry_gate_met: bool,
    pub(crate) dte_selection: String,
    pub(crate) lot_size: u32,
    pub(crate) legs: Vec<ResolvedLeg>,
    pub(crate) problems: Vec<LegProblem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sizing: Option<SizingProblem>,
    pub(crate) would_enter_now: bool,
}

pub(crate) fn resolve_strategy(
    strategy: &Strategy,
    table: &OptionTable,
    reference: impl Fn(&str) -> Option<f64>,
) -> Resolution {
    let mut legs = Vec::with_capacity(strategy.legs.len());
    let mut problems = Vec::new();

    for (index, leg) in strategy.legs.iter().enumerate() {
        match resolve(table, leg.side, &leg.strike) {
            Ok(strike) => {
                let premium = strike.ltp;
                let priced = premium.is_finite() && premium > 0.0;
                let is_short = leg.action == strategy::Action::Sell;

                let stop_price = leg.stop_loss.filter(|_| priced).map(|rule| {
                    let points = rule.points_from(premium);
                    if is_short {
                        premium + points
                    } else {
                        premium - points
                    }
                });
                let target_price = leg.target.filter(|_| priced).map(|rule| {
                    let points = rule.points_from(premium);
                    if is_short {
                        (premium - points).max(0.0)
                    } else {
                        premium + points
                    }
                });

                legs.push(ResolvedLeg {
                    leg: index,
                    action: leg.action,
                    lots: leg.lots,
                    quantity: leg.lots * table.lot_size,
                    stop_price,
                    target_price,
                    strike,
                });
            }
            Err(error) => problems.push(LegProblem {
                leg: index,
                side: leg.side.label(),
                error,
            }),
        }
    }

    let now = ist_minutes_now();
    let entry_condition_met = strategy.entry_condition.is_met(reference);
    let entry_window_open = strategy.entry_open(now);
    let before_hard_exit = strategy.holdable(now);
    let complete = problems.is_empty() && !legs.is_empty();

    let days_to_expiry = ist_today()
        .ok()
        .and_then(|today| days_between(&today, &table.expiry).ok());
    let gate_met = strategy.dte.allows(days_to_expiry);

    // A chain with no lot size cannot size an order: every quantity would be zero.
    let sizing = (table.lot_size == 0).then(|| SizingProblem {
        problem: "chain reports no lot size",
        underlying: strategy.underlying.clone(),
    });

    Resolution {
        id: strategy.id.clone(),
        underlying: strategy.underlying.clone(),
        expiry: table.expiry.clone(),
        spot_price: table.spot_price,
        spot_atm: crate::option_chain::metrics::spot_atm(table),
        entry_condition_met,
        entry_window_open,
        entry_closes_at: strategy.entry_closes_at().to_string(),
        before_hard_exit,
        minutes_until_exit: i64::from(strategy.exit_time.minutes()) - i64::from(now),
        days_to_expiry,
        expiry_gate_met: gate_met,
        dte_selection: strategy.dte.describe(),
        lot_size: table.lot_size,
        legs,
        problems,
        would_enter_now: complete
            && entry_condition_met
            && entry_window_open
            && gate_met
            && sizing.is_none(),
        sizing,
    }
}
