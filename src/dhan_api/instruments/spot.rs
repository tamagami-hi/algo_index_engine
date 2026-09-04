//! Resolving each option chain's underlying to the instrument whose price centres it.
//!
//! Every chain needs a live spot to place ATM. Which instrument supplies that spot is
//! not stated anywhere in the master as a usable link, so it is resolved here.
//!
//! WHY `UNDERLYING_SECURITY_ID` IS NOT USED.
//! The master has an `UNDERLYING_SECURITY_ID` column that looks exactly like the
//! foreign key for this job. It is not usable for index options. Checked against the
//! live master: every `OPTIDX` underlying disagrees with the matching `INDEX` row —
//!
//! | underlying | `INDEX` row | `UNDERLYING_SECURITY_ID` |
//! |------------|-------------|--------------------------|
//! | NIFTY      | 13          | 26000                    |
//! | BANKNIFTY  | 25          | 26009                    |
//! | FINNIFTY   | 27          | 26037                    |
//! | SENSEX     | 51          | 1                        |
//! | BANKEX     | 69          | 12                       |
//!
//! and `26000` does not exist as an instrument row at all, so it cannot be subscribed.
//! It appears to be an exchange-internal id space. Joining on `UNDERLYING_SYMBOL`
//! instead resolves every F&O underlying in the master.

use std::collections::BTreeMap;

use super::master::{ChainKind, InstrumentMaster, SpotKind, SpotRow, UnderlyingKey};
use super::segments::{ExchangeSegment, exchange_id_of};

/// The instrument to subscribe for an underlying's spot price.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SpotInstrument {
    pub(crate) segment: ExchangeSegment,
    pub(crate) security_id: String,
    /// Which rule supplied it, so a future-as-proxy is visible rather than implied.
    pub(crate) kind: SpotKind,
}

/// Outcome of resolving every chain's underlying.
#[derive(Clone, Debug, Default)]
pub(crate) struct SpotResolution {
    pub(crate) resolved: BTreeMap<UnderlyingKey, SpotInstrument>,
    /// Underlyings with no usable spot. These cannot be centred, so they are dropped
    /// from the plan and listed here rather than silently disappearing.
    pub(crate) unresolved: Vec<UnderlyingKey>,
}

/// Resolve the spot instrument for each underlying in `underlyings`.
///
/// `as_of` is an IST `YYYY-MM-DD`, used to reject expired index futures when one is
/// needed as a stand-in.
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

    // Candidate spot rows on the same exchange, of the right sort for this chain.
    let candidates = master.spots.iter().filter(|row| {
        row.exchange_id == exchange_id
            && match kind {
                ChainKind::Stock => row.kind == SpotKind::Equity,
                ChainKind::Index => row.kind == SpotKind::Index,
            }
    });

    // Exact symbol first, then the normalized form. Exact is tried first so a symbol
    // that only matches after stripping punctuation can never win over a literal
    // match, which is what keeps look-alike tickers apart.
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

    // An index with no INDEX row of its own. Dhan lists option chains on indices it
    // publishes no spot value for, so the nearest live index FUTURE stands in. It is a
    // proxy, not the index: it carries basis, which is acceptable for choosing which
    // strike is ATM but is recorded as IndexFuture so no caller mistakes it for spot.
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

fn spot_of(row: &SpotRow) -> SpotInstrument {
    SpotInstrument {
        segment: row.segment,
        security_id: row.security_id.clone(),
        kind: row.kind,
    }
}

/// Whether a spot row names the wanted underlying.
///
/// Index rows are matched on `SYMBOL_NAME` as well: the two columns agree for every
/// index in the current master, but they are populated independently and only one of
/// them needs to carry the F&O name for the join to succeed.
fn symbol_matches(wanted: &str, row: &SpotRow) -> bool {
    normalize_symbol(&row.underlying_symbol) == wanted
        || (row.kind == SpotKind::Index && normalize_symbol(&row.symbol_name) == wanted)
}

/// Upper-case, alphanumeric-only form of a symbol.
///
/// Dhan writes the same underlying with and without punctuation and spacing across
/// columns (`BAJAJ-AUTO`, `NIFTY NEXT 50`), so the join key ignores both.
fn normalize_symbol(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_uppercase())
        .collect()
}
