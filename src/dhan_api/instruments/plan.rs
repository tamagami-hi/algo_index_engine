use std::collections::BTreeMap;

use anyhow::{Result, bail};

use super::chain::{OptionChain, index_option_chains, select_window};
use super::master::{ChainKind, InstrumentMaster, UnderlyingKey, to_strike_units};
use super::spot::{SpotInstrument, resolve_spots};
use super::subscription::{MAX_INSTRUMENTS, Subscription};

#[derive(Clone, Copy, Debug)]
pub(crate) struct PlanConfig {
    pub(crate) strikes_each_side: usize,
    pub(crate) max_instruments: usize,
}

impl Default for PlanConfig {
    fn default() -> Self {
        Self {
            strikes_each_side: 2,
            max_instruments: MAX_INSTRUMENTS,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SpotRoute {
    pub(crate) subscription: Subscription,
    pub(crate) underlyings: Vec<UnderlyingKey>,
}

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

#[derive(Clone, Debug, Default)]
pub(crate) struct SubscriptionPlan {
    pub(crate) spot_subscriptions: Vec<Subscription>,
    pub(crate) option_subscriptions: Vec<Subscription>,
    pub(crate) spot_routes: Vec<SpotRoute>,
    pub(crate) chains: Vec<SelectedChain>,
    pub(crate) skipped_for_budget: Vec<UnderlyingKey>,
    pub(crate) unresolved_underlyings: Vec<UnderlyingKey>,
}

impl SubscriptionPlan {
    pub(crate) fn len(&self) -> usize {
        self.spot_subscriptions.len() + self.option_subscriptions.len()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ChainUniverse {
    pub(crate) chains: BTreeMap<UnderlyingKey, OptionChain>,
    pub(crate) spots: BTreeMap<UnderlyingKey, SpotInstrument>,
    pub(crate) unresolved: Vec<UnderlyingKey>,
    pub(crate) as_of: String,
}

impl ChainUniverse {
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

pub(crate) fn build_plan(
    universe: &ChainUniverse,
    spot_prices: &BTreeMap<UnderlyingKey, f64>,
    config: PlanConfig,
) -> Result<SubscriptionPlan> {
    let mut chains = Vec::new();
    let mut option_subscriptions = Vec::new();
    let mut skipped_for_budget = Vec::new();
    let mut priced_keys = Vec::new();
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
