use std::collections::BTreeMap;

use super::master::{ChainKind, InstrumentMaster, SpotKind, SpotRow, UnderlyingKey};
use super::segments::{ExchangeSegment, exchange_id_of};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SpotInstrument {
    pub(crate) segment: ExchangeSegment,
    pub(crate) security_id: String,
    pub(crate) kind: SpotKind,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SpotResolution {
    pub(crate) resolved: BTreeMap<UnderlyingKey, SpotInstrument>,
    pub(crate) unresolved: Vec<UnderlyingKey>,
}

pub(crate) fn resolve_spots<'a>(
    master: &InstrumentMaster,
    underlyings: impl IntoIterator<Item = (&'a UnderlyingKey, ChainKind)>,
    as_of: &str,
) -> SpotResolution {
    let mut resolution = SpotResolution::default();

    for (key, kind) in underlyings {
        match resolve_one(master, key, kind, as_of) {
            Some(spot) => {
                resolution.resolved.insert(key.clone(), spot);
            }
            None => resolution.unresolved.push(key.clone()),
        }
    }
    resolution
}

fn resolve_one(
    master: &InstrumentMaster,
    key: &UnderlyingKey,
    kind: ChainKind,
    as_of: &str,
) -> Option<SpotInstrument> {
    let exchange_id = exchange_id_of(key.segment).ok()?;
    let wanted = normalize_symbol(&key.symbol);

    let candidates = master.spots.iter().filter(|row| {
        row.exchange_id == exchange_id
            && match kind {
                ChainKind::Stock => row.kind == SpotKind::Equity,
                ChainKind::Index => row.kind == SpotKind::Index,
            }
    });

    let mut normalized_match = None;
    for row in candidates {
        if row.underlying_symbol == key.symbol {
            return Some(spot_of(row));
        }
        if normalized_match.is_none() && symbol_matches(&wanted, row) {
            normalized_match = Some(row);
        }
    }
    if let Some(row) = normalized_match {
        return Some(spot_of(row));
    }

    if kind == ChainKind::Index {
        return master
            .spots
            .iter()
            .filter(|row| {
                row.kind == SpotKind::IndexFuture
                    && row.exchange_id == exchange_id
                    && normalize_symbol(&row.underlying_symbol) == wanted
                    && row.expiry.as_str() >= as_of
            })
            .min_by(|left, right| left.expiry.cmp(&right.expiry))
            .map(spot_of);
    }
    None
}

pub(crate) fn resolve_index_spot(master: &InstrumentMaster, symbol: &str) -> Option<SpotInstrument> {
    let wanted = normalize_symbol(symbol);
    master
        .spots
        .iter()
        .find(|row| {
            row.kind == SpotKind::Index
                && (normalize_symbol(&row.underlying_symbol) == wanted
                    || normalize_symbol(&row.symbol_name) == wanted)
        })
        .map(spot_of)
}

fn spot_of(row: &SpotRow) -> SpotInstrument {
    SpotInstrument {
        segment: row.segment,
        security_id: row.security_id.clone(),
        kind: row.kind,
    }
}

fn symbol_matches(wanted: &str, row: &SpotRow) -> bool {
    normalize_symbol(&row.underlying_symbol) == wanted
        || (row.kind == SpotKind::Index && normalize_symbol(&row.symbol_name) == wanted)
}

fn normalize_symbol(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_uppercase())
        .collect()
}
