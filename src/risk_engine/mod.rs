pub(crate) mod store;
pub(crate) mod strategy;
pub(crate) mod strike;

use serde::Serialize;

use crate::dhan_api::instruments::{days_between, ist_minutes_now, ist_today};
use crate::option_chain::quality::{
    self, QuoteProblem, QuoteSide, UsableQuote, freshness, usable_price,
};
use crate::option_chain::table::OptionTable;
use strategy::Strategy;
use strike::{ResolvedStrike, StrikeError, resolve};

#[cfg(test)]
#[path = "../../tests/risk_engine/gate.rs"]
mod tests;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FeedHealth {
    pub(crate) connected: bool,
    pub(crate) last_frame_at: Option<u64>,
}

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
    pub(crate) needs_side: QuoteSide,
    pub(crate) quote_age_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct LegProblem {
    pub(crate) leg: usize,
    pub(crate) side: &'static str,
    #[serde(flatten)]
    pub(crate) error: StrikeError,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "blocker", rename_all = "snake_case")]
pub(crate) enum Blocker {
    FeedDisconnected,
    FeedSilent {
        age_ms: u64,
        limit_ms: u64,
    },
    UnderlyingNeverQuoted {
        underlying: String,
    },
    UnderlyingStale {
        age_ms: u64,
        limit_ms: u64,
    },
    UnderlyingPriceUnusable,
    ContractUnresolved {
        leg: usize,
    },
    NoLegs,
    ChainHasNoLotSize,
    OptionQuote {
        leg: usize,
        side: &'static str,
        detail: QuoteProblem,
    },
    OutsideEntryWindow {
        now: String,
        opens: String,
        closes: String,
    },
    EntryConditionNotMet,
    DteNotSelected {
        days_to_expiry: Option<i64>,
        selection: String,
    },
}

impl Blocker {
    pub(crate) fn describe(&self) -> String {
        match self {
            Self::FeedDisconnected => "the market feed is not connected".to_owned(),
            Self::FeedSilent { age_ms, limit_ms } => {
                format!("no feed frame for {age_ms}ms, limit {limit_ms}ms")
            }
            Self::UnderlyingNeverQuoted { underlying } => {
                format!("{underlying} has not quoted yet")
            }
            Self::UnderlyingStale { age_ms, limit_ms } => {
                format!("the underlying is {age_ms}ms old, limit {limit_ms}ms")
            }
            Self::UnderlyingPriceUnusable => "the underlying price is not usable".to_owned(),
            Self::ContractUnresolved { leg } => format!("leg {} has no contract", leg + 1),
            Self::NoLegs => "the strategy has no legs".to_owned(),
            Self::ChainHasNoLotSize => "the chain reports no lot size".to_owned(),
            Self::OptionQuote { leg, side, detail } => {
                let reason = match detail {
                    QuoteProblem::NeverQuoted => "has not quoted yet".to_owned(),
                    QuoteProblem::Stale { age_ms, limit_ms } => {
                        format!("is {age_ms}ms old, limit {limit_ms}ms")
                    }
                    QuoteProblem::PremiumUnavailable => "has no usable premium".to_owned(),
                    QuoteProblem::NoUsableSide { side } => {
                        format!("has no usable {}", side.label())
                    }
                };
                format!("leg {} ({side}) {reason}", leg + 1)
            }
            Self::OutsideEntryWindow { now, opens, closes } => {
                format!("{now} is outside the entry window {opens} to {closes}")
            }
            Self::EntryConditionNotMet => "the entry condition is not met".to_owned(),
            Self::DteNotSelected {
                days_to_expiry,
                selection,
            } => match days_to_expiry {
                Some(days) => format!("{days}DTE is not in the selection {selection}"),
                None => "the days to expiry are unknown".to_owned(),
            },
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Resolution {
    pub(crate) id: String,
    pub(crate) underlying: String,
    pub(crate) expiry: String,
    pub(crate) spot_price: f64,
    pub(crate) spot_atm: Option<f64>,
    pub(crate) spot_age_ms: Option<u64>,
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
    pub(crate) blockers: Vec<Blocker>,
    pub(crate) blocked_because: Vec<String>,
    pub(crate) would_enter_now: bool,
}

fn needed_side(action: strategy::Action) -> QuoteSide {
    match action {
        strategy::Action::Sell => QuoteSide::Bid,
        strategy::Action::Buy => QuoteSide::Ask,
    }
}

pub(crate) fn resolve_strategy(
    strategy: &Strategy,
    table: &OptionTable,
    feed: FeedHealth,
    reference: impl Fn(&str) -> Option<f64>,
) -> Resolution {
    let limits = freshness();
    let mut blockers = Vec::new();

    if !feed.connected {
        blockers.push(Blocker::FeedDisconnected);
    }
    match feed.last_frame_at {
        None => {}
        Some(stamp) => {
            let age_ms = quality::age_since(stamp);
            if age_ms > limits.feed_silence_max_ms {
                blockers.push(Blocker::FeedSilent {
                    age_ms,
                    limit_ms: limits.feed_silence_max_ms,
                });
            }
        }
    }

    let spot_age_ms = (table.spot_updates > 0).then(|| quality::age_since(table.spot_received_at));
    match spot_age_ms {
        None => blockers.push(Blocker::UnderlyingNeverQuoted {
            underlying: table.underlying.symbol.clone(),
        }),
        Some(age_ms) if age_ms > limits.underlying_max_age_ms => {
            blockers.push(Blocker::UnderlyingStale {
                age_ms,
                limit_ms: limits.underlying_max_age_ms,
            });
        }
        Some(_) if usable_price(table.spot_price).is_none() => {
            blockers.push(Blocker::UnderlyingPriceUnusable);
        }
        Some(_) => {}
    }

    let mut legs = Vec::with_capacity(strategy.legs.len());
    let mut problems = Vec::new();

    for (index, leg) in strategy.legs.iter().enumerate() {
        let needs_side = needed_side(leg.action);
        match resolve(table, leg.side, &leg.strike) {
            Ok(strike) => {
                let raw = match leg.side {
                    strike::Side::Call => table.calls.raw_quote(strike.row),
                    strike::Side::Put => table.puts.raw_quote(strike.row),
                };
                let inspected = quality::inspect_quote(raw, Some(needs_side));
                if let Err(detail) = inspected {
                    blockers.push(Blocker::OptionQuote {
                        leg: index,
                        side: leg.side.label(),
                        detail,
                    });
                }

                let usable: Option<UsableQuote> = inspected.ok();
                let premium = usable.map(|quote| quote.premium);
                let is_short = leg.action == strategy::Action::Sell;

                let stop_price = premium.zip(leg.stop_loss).map(|(premium, rule)| {
                    let points = rule.points_from(premium);
                    if is_short {
                        premium + points
                    } else {
                        premium - points
                    }
                });
                let target_price = premium.zip(leg.target).map(|(premium, rule)| {
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
                    needs_side,
                    quote_age_ms: usable.map(|quote| quote.age_ms),
                    strike,
                });
            }
            Err(error) => {
                problems.push(LegProblem {
                    leg: index,
                    side: leg.side.label(),
                    error,
                });
                blockers.push(Blocker::ContractUnresolved { leg: index });
            }
        }
    }

    if strategy.legs.is_empty() {
        blockers.push(Blocker::NoLegs);
    }

    if table.lot_size == 0 {
        blockers.push(Blocker::ChainHasNoLotSize);
    }

    let now = ist_minutes_now();
    let entry_condition_met = strategy.entry_condition.is_met(reference);
    if !entry_condition_met {
        blockers.push(Blocker::EntryConditionNotMet);
    }

    let entry_window_open = strategy.entry_open(now);
    if !entry_window_open {
        blockers.push(Blocker::OutsideEntryWindow {
            now: strategy::TimeOfDay::from_minutes(now).to_string(),
            opens: strategy.entry_time.to_string(),
            closes: strategy.entry_closes_at().to_string(),
        });
    }
    let before_hard_exit = strategy.holdable(now);

    let days_to_expiry = ist_today()
        .ok()
        .and_then(|today| days_between(&today, &table.expiry).ok());
    let gate_met = strategy.dte.allows(days_to_expiry);
    if !gate_met {
        blockers.push(Blocker::DteNotSelected {
            days_to_expiry,
            selection: strategy.dte.describe(),
        });
    }

    let blocked_because = blockers.iter().map(Blocker::describe).collect();

    Resolution {
        id: strategy.id.clone(),
        underlying: strategy.underlying.clone(),
        expiry: table.expiry.clone(),
        spot_price: table.spot_price,
        spot_atm: crate::option_chain::metrics::spot_atm(table),
        spot_age_ms,
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
        would_enter_now: blockers.is_empty(),
        blockers,
        blocked_because,
    }
}
