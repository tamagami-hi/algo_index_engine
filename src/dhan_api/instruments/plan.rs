//! Assembling the list of instruments to subscribe.
//!
//! A Dhan subscription is a `(ExchangeSegment, SecurityId)` pair — that is the whole
//! addressing scheme, per the v2 Live Market Feed docs. There is no instrument-token
//! concept to translate into, so [`Subscription`] is deliberately just those two
//! fields, ready to serialise into `InstrumentList`.
//!
//! THE PLAN IS BUILT IN TWO STAGES, because ATM depends on a price:
//!
//! ```text
//!   master CSV ──> chains + spot instruments ──> [`discovery_plan`]   (no prices yet)
//!                                                      │
//!                                       subscribe spots, collect LTPs
//!                                                      │
//!                                                      v
//!                                              [`build_plan`]         (legs chosen)
//! ```
//!
//! Stage one is everything that can be known from the CSV alone, which is what the
//! caller needs before opening a feed. Stage two needs the spot prices stage one
//! collects.

use std::collections::BTreeMap;

use anyhow::{Result, bail};

use super::chain::{OptionChain, index_option_chains, select_window};
use super::master::{ChainKind, InstrumentMaster, UnderlyingKey, to_strike_units};
use super::segments::ExchangeSegment;
use super::spot::{SpotInstrument, resolve_spots};

/// Dhan allows five feed connections of 5,000 instruments each.
pub(crate) const MAX_INSTRUMENTS_PER_CONNECTION: usize = 5_000;
pub(crate) const MAX_CONNECTIONS: usize = 5;
/// The account-wide ceiling the plan must respect.
pub(crate) const MAX_INSTRUMENTS_PER_ACCOUNT: usize =
    MAX_INSTRUMENTS_PER_CONNECTION * MAX_CONNECTIONS;

/// One instrument, in exactly the form Dhan's subscribe message needs.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct Subscription {
    pub(crate) segment: ExchangeSegment,
    pub(crate) security_id: String,
}

/// How wide a window to monitor, and how many instruments may be spent on it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PlanConfig {
    /// Strikes each side of ATM. `2` gives five strikes, so ten option legs.
    pub(crate) strikes_each_side: usize,
    /// Instrument ceiling for the whole plan.
    pub(crate) max_instruments: usize,
}

impl Default for PlanConfig {
    fn default() -> Self {
        Self {
            strikes_each_side: 2,
            max_instruments: MAX_INSTRUMENTS_PER_ACCOUNT,
        }
    }
}

/// A spot instrument and the underlyings whose ATM it determines.
///
/// A list, not a single key, because one spot row can centre more than one chain.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SpotRoute {
    pub(crate) subscription: Subscription,
    pub(crate) underlyings: Vec<UnderlyingKey>,
}

/// One underlying's monitored window.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SelectedChain {
    pub(crate) underlying: UnderlyingKey,
    pub(crate) kind: ChainKind,
    pub(crate) expiry: String,
    pub(crate) lot_size: u32,
    pub(crate) spot: SpotInstrument,
    pub(crate) spot_price: f64,
    pub(crate) atm_strike: f64,
    pub(crate) legs: Vec<Subscription>,
}

/// The finished instrument list.
#[derive(Clone, Debug, Default)]
pub(crate) struct SubscriptionPlan {
    /// Spot instruments. Ticker mode is sufficient — only the last price is needed to
    /// place ATM, so asking for Quote or Full here would spend bandwidth on depth
    /// nobody reads.
    pub(crate) spot_subscriptions: Vec<Subscription>,
    /// Option legs. These want Full mode, which is the only mode carrying open
    /// interest and five-level depth in one packet.
    pub(crate) option_subscriptions: Vec<Subscription>,
    pub(crate) spot_routes: Vec<SpotRoute>,
    pub(crate) chains: Vec<SelectedChain>,
    /// Underlyings dropped because the instrument ceiling was reached.
    pub(crate) skipped_for_budget: Vec<UnderlyingKey>,
    /// Underlyings with no resolvable spot instrument.
    pub(crate) unresolved_underlyings: Vec<UnderlyingKey>,
}

impl SubscriptionPlan {
    /// Total instruments the plan subscribes.
    pub(crate) fn len(&self) -> usize {
        self.spot_subscriptions.len() + self.option_subscriptions.len()
    }

    /// Feed connections this plan needs, at 5,000 instruments each.
    pub(crate) fn connections_required(&self) -> usize {
        self.len().div_ceil(MAX_INSTRUMENTS_PER_CONNECTION)
    }
}

/// Every option chain in the master, with its spot instrument resolved.
///
/// The expensive part of planning, and independent of any price, so it is done once and
/// reused for every re-centre.
#[derive(Clone, Debug)]
pub(crate) struct ChainUniverse {
    pub(crate) chains: BTreeMap<UnderlyingKey, OptionChain>,
    pub(crate) spots: BTreeMap<UnderlyingKey, SpotInstrument>,
    pub(crate) unresolved: Vec<UnderlyingKey>,
    pub(crate) as_of: String,
}

impl ChainUniverse {
    /// Index the master into chains and resolve each chain's spot instrument.
    pub(crate) fn build(master: &InstrumentMaster, as_of: &str) -> Result<Self> {
        let chains = index_option_chains(master, as_of);
        if chains.is_empty() {
            bail!("no live option chains in the instrument master as of {as_of}");
        }

        let resolution = resolve_spots(
            master,
            chains.iter().map(|(key, chain)| (key, chain.kind)),
            as_of,
        );
        Ok(Self {
            chains,
            spots: resolution.resolved,
            unresolved: resolution.unresolved,
            as_of: as_of.to_owned(),
        })
    }

    /// Underlyings in the order they should win a contested instrument budget.
    ///
    /// Indices first, then stocks, alphabetical within each group. Indices come first
    /// because they are the most liquid option books on the exchange, so if the
    /// ceiling binds, they are the ones worth keeping. Alphabetical within a group
    /// makes the cut deterministic run to run instead of depending on map order.
    fn prioritised(&self) -> Vec<&UnderlyingKey> {
        let mut keys: Vec<&UnderlyingKey> = self
            .chains
            .keys()
            .filter(|key| self.spots.contains_key(*key))
            .collect();
        keys.sort_by(|left, right| {
            let index_first = |key: &UnderlyingKey| {
                match self.chains.get(key).map(|chain| chain.kind) {
                    Some(ChainKind::Index) => 0,
                    _ => 1,
                }
            };
            index_first(left)
                .cmp(&index_first(right))
                .then_with(|| left.symbol.cmp(&right.symbol))
                .then_with(|| left.segment.cmp(&right.segment))
        });
        keys
    }
}

/// Stage one: the spot instruments to subscribe before any price is known.
///
/// This is the list that has to be ready before the feed opens. It carries no option
/// legs, because which strikes are at the money is not yet knowable.
pub(crate) fn discovery_plan(universe: &ChainUniverse) -> SubscriptionPlan {
    let (spot_subscriptions, spot_routes) = spot_routes_of(universe, universe.chains.keys());

    SubscriptionPlan {
        spot_subscriptions,
        option_subscriptions: Vec::new(),
        spot_routes,
        chains: Vec::new(),
        skipped_for_budget: Vec::new(),
        unresolved_underlyings: universe.unresolved.clone(),
    }
}

/// Stage two: add the option legs, given the spot prices collected from stage one.
///
/// Underlyings with no price are left out rather than guessed at — an ATM placed on a
/// stale or invented price would monitor the wrong strikes, which is worse than
/// monitoring none.
pub(crate) fn build_plan(
    universe: &ChainUniverse,
    spot_prices: &BTreeMap<UnderlyingKey, f64>,
    config: PlanConfig,
) -> Result<SubscriptionPlan> {
    let mut chains = Vec::new();
    let mut option_subscriptions = Vec::new();
    let mut skipped_for_budget = Vec::new();
    let mut priced_keys = Vec::new();
    // Every chain costs its legs plus the one spot that keeps it centred.
    let mut used = 0;

    for key in universe.prioritised() {
        let Some(price) = spot_prices.get(key).copied() else {
            continue;
        };
        if !price.is_finite() || price <= 0.0 {
            bail!("invalid spot price {price} for {}", key.symbol);
        }
        let Some(chain) = universe.chains.get(key) else {
            continue;
        };
        let Some(spot) = universe.spots.get(key) else {
            continue;
        };
        let Some(window) = select_window(
            &chain.strikes,
            to_strike_units(price),
            config.strikes_each_side,
        ) else {
            continue;
        };

        let legs: Vec<Subscription> = chain
            .legs(&window.strikes)
            .into_iter()
            .map(|contract| Subscription {
                segment: contract.segment,
                security_id: contract.security_id.clone(),
            })
            .collect();
        if legs.is_empty() {
            continue;
        }

        let cost = legs.len() + 1;
        if used + cost > config.max_instruments {
            skipped_for_budget.push(key.clone());
            continue;
        }
        used += cost;

        option_subscriptions.extend(legs.iter().cloned());
        priced_keys.push(key);
        chains.push(SelectedChain {
            underlying: key.clone(),
            kind: chain.kind,
            expiry: chain.expiry.clone(),
            lot_size: chain.lot_size,
            spot: spot.clone(),
            spot_price: price,
            atm_strike: window.atm_strike(),
            legs,
        });
    }

    if chains.is_empty() {
        bail!("no option chain could be centred on a live underlying price");
    }

    let (spot_subscriptions, spot_routes) = spot_routes_of(universe, priced_keys.into_iter());
    option_subscriptions.sort();
    option_subscriptions.dedup();

    let plan = SubscriptionPlan {
        spot_subscriptions,
        option_subscriptions,
        spot_routes,
        chains,
        skipped_for_budget,
        unresolved_underlyings: universe.unresolved.clone(),
    };
    if plan.len() > config.max_instruments {
        bail!(
            "plan holds {} instruments, above the {} ceiling",
            plan.len(),
            config.max_instruments
        );
    }
    Ok(plan)
}

/// Deduplicate the spot instruments for `keys`, keeping which underlyings each prices.
///
/// Deduplication matters: one spot row may centre several chains, and subscribing it
/// twice would waste a slot out of the account's 25,000.
fn spot_routes_of<'a>(
    universe: &ChainUniverse,
    keys: impl IntoIterator<Item = &'a UnderlyingKey>,
) -> (Vec<Subscription>, Vec<SpotRoute>) {
    let mut by_instrument: BTreeMap<Subscription, Vec<UnderlyingKey>> = BTreeMap::new();

    for key in keys {
        let Some(spot) = universe.spots.get(key) else {
            continue;
        };
        by_instrument
            .entry(Subscription {
                segment: spot.segment,
                security_id: spot.security_id.clone(),
            })
            .or_default()
            .push(key.clone());
    }

    let subscriptions: Vec<Subscription> = by_instrument.keys().cloned().collect();
    let routes = by_instrument
        .into_iter()
        .map(|(subscription, underlyings)| SpotRoute {
            subscription,
            underlyings,
        })
        .collect();
    (subscriptions, routes)
}
