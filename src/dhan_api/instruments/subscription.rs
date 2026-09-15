use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};

use super::master::{ChainKind, InstrumentMaster, OptionContract, SpotKind, UnderlyingKey};
use super::segments::ExchangeSegment;
use super::spot::resolve_spots;

pub(crate) const MAX_PER_MESSAGE: usize = 100;
pub(crate) const MAX_PER_CONNECTION: usize = 5_000;
pub(crate) const MAX_CONNECTIONS: usize = 5;
pub(crate) const MAX_INSTRUMENTS: usize = MAX_PER_CONNECTION * MAX_CONNECTIONS;

const EXCLUDED_INDEX_CHAINS: &[(ExchangeSegment, &str)] = &[
    (ExchangeSegment::BseFno, "SENSEX50"),
    (ExchangeSegment::McxComm, "MCXBULLDEX"),
];

fn is_excluded(segment: ExchangeSegment, symbol: &str) -> bool {
    EXCLUDED_INDEX_CHAINS
        .iter()
        .any(|(excluded_segment, excluded_symbol)| {
            *excluded_segment == segment && *excluded_symbol == symbol
        })
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct Subscription {
    pub(crate) segment: ExchangeSegment,
    pub(crate) security_id: String,
}

#[derive(Clone, Debug)]
pub(crate) struct Pool {
    pub(crate) label: &'static str,
    pub(crate) instruments: Vec<Subscription>,
}

impl Pool {
    pub(crate) fn len(&self) -> usize {
        self.instruments.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.instruments.is_empty()
    }

    pub(crate) fn messages(&self) -> std::slice::Chunks<'_, Subscription> {
        self.instruments.chunks(MAX_PER_MESSAGE)
    }

    pub(crate) fn message_count(&self) -> usize {
        self.instruments.len().div_ceil(MAX_PER_MESSAGE)
    }

    pub(crate) fn connections_required(&self) -> usize {
        self.instruments.len().div_ceil(MAX_PER_CONNECTION)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ChainSummary {
    pub(crate) underlying: UnderlyingKey,
    pub(crate) expiry: String,
    pub(crate) contracts: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct CatalogReport {
    pub(crate) as_of: String,
    pub(crate) index_underlyings: usize,
    pub(crate) stock_underlyings: usize,
    pub(crate) spot_index: usize,
    pub(crate) spot_equity: usize,
    pub(crate) spot_index_future: usize,
    pub(crate) unresolved: Vec<UnderlyingKey>,
    pub(crate) index_chains: Vec<ChainSummary>,
    pub(crate) excluded_index_chains: Vec<ChainSummary>,
}

#[derive(Clone, Debug)]
pub(crate) struct Catalog {
    pub(crate) spot: Pool,
    pub(crate) index_options: Pool,
    pub(crate) report: CatalogReport,
}

impl Catalog {
    pub(crate) fn len(&self) -> usize {
        self.spot.len() + self.index_options.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.spot.is_empty() && self.index_options.is_empty()
    }

    pub(crate) fn pools(&self) -> [&Pool; 2] {
        [&self.spot, &self.index_options]
    }

    pub(crate) fn message_count(&self) -> usize {
        self.spot.message_count() + self.index_options.message_count()
    }

    pub(crate) fn connections_required(&self) -> usize {
        self.len().div_ceil(MAX_PER_CONNECTION)
    }
}

pub(crate) fn build_catalog(master: &InstrumentMaster, as_of: &str) -> Result<Catalog> {
    let kinds = live_underlyings(master, as_of);
    if kinds.is_empty() {
        bail!("no live option underlyings in the instrument master as of {as_of}");
    }

    let index_underlyings = kinds
        .values()
        .filter(|kind| **kind == ChainKind::Index)
        .count();
    let stock_underlyings = kinds.len() - index_underlyings;

    let resolution = resolve_spots(master, kinds.iter().map(|(key, kind)| (key, *kind)), as_of);

    let mut spot_index = 0;
    let mut spot_equity = 0;
    let mut spot_index_future = 0;
    let mut spot_seen: BTreeSet<Subscription> = BTreeSet::new();
    for instrument in resolution.resolved.values() {
        let subscription = Subscription {
            segment: instrument.segment,
            security_id: instrument.security_id.clone(),
        };
        if spot_seen.insert(subscription) {
            match instrument.kind {
                SpotKind::Index => spot_index += 1,
                SpotKind::Equity => spot_equity += 1,
                SpotKind::IndexFuture => spot_index_future += 1,
            }
        }
    }

    let front = front_expiries(master, as_of);
    let selected = front_index_contracts(master, as_of, &front);

    let mut counts: BTreeMap<(UnderlyingKey, String), usize> = BTreeMap::new();
    let mut excluded_counts: BTreeMap<(UnderlyingKey, String), usize> = BTreeMap::new();
    let mut option_seen: BTreeSet<Subscription> = BTreeSet::new();
    let mut option_instruments = Vec::with_capacity(selected.len());
    for contract in selected {
        let key = UnderlyingKey::new(contract.segment, contract.underlying_symbol.as_str());

        if is_excluded(contract.segment, contract.underlying_symbol.as_str()) {
            *excluded_counts
                .entry((key, contract.expiry.clone()))
                .or_insert(0) += 1;
            continue;
        }

        let subscription = Subscription {
            segment: contract.segment,
            security_id: contract.security_id.clone(),
        };
        if !option_seen.insert(subscription.clone()) {
            continue;
        }
        option_instruments.push(subscription);
        *counts.entry((key, contract.expiry.clone())).or_insert(0) += 1;
    }

    let summarise = |counts: BTreeMap<(UnderlyingKey, String), usize>| -> Vec<ChainSummary> {
        counts
            .into_iter()
            .map(|((underlying, expiry), contracts)| ChainSummary {
                underlying,
                expiry,
                contracts,
            })
            .collect()
    };
    let index_chains = summarise(counts);
    let excluded_index_chains = summarise(excluded_counts);

    let catalog = Catalog {
        spot: Pool {
            label: "spot",
            instruments: spot_seen.into_iter().collect(),
        },
        index_options: Pool {
            label: "index_options",
            instruments: option_instruments,
        },
        report: CatalogReport {
            as_of: as_of.to_owned(),
            index_underlyings,
            stock_underlyings,
            spot_index,
            spot_equity,
            spot_index_future,
            unresolved: resolution.unresolved,
            index_chains,
            excluded_index_chains,
        },
    };

    if catalog.index_options.is_empty() {
        bail!("no live index option contracts in the instrument master as of {as_of}");
    }
    if catalog.len() > MAX_INSTRUMENTS {
        bail!(
            "catalog holds {} instruments, above the {} ceiling ({} connections x {})",
            catalog.len(),
            MAX_INSTRUMENTS,
            MAX_CONNECTIONS,
            MAX_PER_CONNECTION
        );
    }
    Ok(catalog)
}

fn live_underlyings(master: &InstrumentMaster, as_of: &str) -> BTreeMap<UnderlyingKey, ChainKind> {
    let mut kinds = BTreeMap::new();
    for contract in &master.options {
        if contract.expiry.as_str() < as_of {
            continue;
        }
        let key = UnderlyingKey::new(contract.segment, contract.underlying_symbol.as_str());
        kinds.insert(key, contract.kind);
    }
    kinds
}

fn front_expiries(master: &InstrumentMaster, as_of: &str) -> BTreeMap<UnderlyingKey, String> {
    let mut front: BTreeMap<UnderlyingKey, String> = BTreeMap::new();
    for contract in &master.options {
        if contract.expiry.as_str() < as_of {
            continue;
        }
        let key = UnderlyingKey::new(contract.segment, contract.underlying_symbol.as_str());
        match front.get(&key) {
            Some(current) if current.as_str() <= contract.expiry.as_str() => {}
            _ => {
                front.insert(key, contract.expiry.clone());
            }
        }
    }
    front
}

fn front_index_contracts<'master>(
    master: &'master InstrumentMaster,
    as_of: &str,
    front: &BTreeMap<UnderlyingKey, String>,
) -> Vec<&'master OptionContract> {
    let mut selected: Vec<&OptionContract> = master
        .options
        .iter()
        .filter(|contract| contract.kind == ChainKind::Index && contract.expiry.as_str() >= as_of)
        .filter(|contract| {
            let key = UnderlyingKey::new(contract.segment, contract.underlying_symbol.as_str());
            front
                .get(&key)
                .is_some_and(|expiry| expiry.as_str() == contract.expiry.as_str())
        })
        .collect();

    selected.sort_by(|left, right| {
        left.segment
            .cmp(&right.segment)
            .then_with(|| left.underlying_symbol.cmp(&right.underlying_symbol))
            .then_with(|| left.strike_units.cmp(&right.strike_units))
            .then_with(|| left.option_type.cmp(&right.option_type))
    });
    selected
}

#[cfg(test)]
#[path = "../../../tests/dhan_api/instruments/subscription.rs"]
mod tests;
