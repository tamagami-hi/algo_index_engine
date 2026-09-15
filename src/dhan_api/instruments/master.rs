//! Reading the Dhan instrument master CSV.
//!
//! Source: `https://images.dhan.co/api-data/api-scrip-master-detailed.csv`, saved by
//! `dhan_api::instrument_dl`. Roughly 200k rows covering every instrument Dhan
//! can trade.
//!
//! COLUMNS ARE LOCATED BY NAME, NEVER BY POSITION.
//! Dhan publishes two masters (plain and "detailed") whose column sets differ, and the
//! detailed one has gained columns over time. Positional indexing would silently shift
//! every field the next time that happens; name lookup makes a new column harmless.
//!
//! Rows this file cannot make sense of are counted and skipped rather than aborting the
//! load. One malformed line out of 200k must not cost the whole universe, but a silent
//! skip is useless for diagnosis, so every reason has a counter in [`ParseReport`].

use std::{fs, path::Path};

use anyhow::{Context, Result, bail};

use super::segments::{ExchangeSegment, derivative_segment, spot_segment};
use super::trading_day::parse_iso_date;

/// Strikes are held as integers to keep them usable as map keys and to make equality
/// exact. Dhan quotes strikes to five decimals (`84600.00000`), so this scale is
/// lossless for every strike in the master.
pub(crate) const STRIKE_SCALE: f64 = 100_000.0;

/// Convert a rupee price to the integer strike units used throughout this module.
pub(crate) fn to_strike_units(price: f64) -> i64 {
    (price * STRIKE_SCALE).round() as i64
}

/// Convert integer strike units back to rupees, for display.
pub(crate) fn from_strike_units(units: i64) -> f64 {
    units as f64 / STRIKE_SCALE
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum OptionType {
    Call,
    Put,
}

/// Whether an option chain sits on a single stock or on an index.
///
/// Indices are worth distinguishing because they are the most liquid option books on
/// the exchange, so they are the ones to keep when a subscription budget binds.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum ChainKind {
    Index,
    Stock,
}

/// What kind of spot reference a row can serve as.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum SpotKind {
    /// An `INDEX` row: the index value itself, fed under `IDX_I`.
    Index,
    /// An `EQUITY` row: the cash share.
    Equity,
    /// A `FUTIDX` row: an index future, used only when an index has no `INDEX` row.
    IndexFuture,
}

/// Identifies one option chain: an underlying within one derivative segment.
///
/// Segment is part of the key because the same underlying name can carry chains on
/// two exchanges (SENSEX on BSE, NIFTY on NSE) and they are separate books.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct UnderlyingKey {
    pub(crate) segment: ExchangeSegment,
    pub(crate) symbol: String,
}

impl UnderlyingKey {
    pub(crate) fn new(segment: ExchangeSegment, symbol: impl Into<String>) -> Self {
        Self {
            segment,
            symbol: symbol.into(),
        }
    }
}

/// One CE or PE contract from the master.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OptionContract {
    pub(crate) segment: ExchangeSegment,
    pub(crate) security_id: String,
    pub(crate) underlying_symbol: String,
    /// `YYYY-MM-DD`. Sorts chronologically as a string, which is how expiries are
    /// compared everywhere downstream.
    pub(crate) expiry: String,
    pub(crate) strike_units: i64,
    pub(crate) option_type: OptionType,
    pub(crate) lot_size: u32,
    pub(crate) kind: ChainKind,
}

/// A row that can act as the spot price reference for an underlying.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SpotRow {
    pub(crate) segment: ExchangeSegment,
    pub(crate) security_id: String,
    pub(crate) exchange_id: String,
    /// The master's `UNDERLYING_SYMBOL`. For an `EQUITY` row this is the ticker
    /// (`RELIANCE`), not the company name — `SYMBOL_NAME` holds the company name.
    pub(crate) underlying_symbol: String,
    pub(crate) symbol_name: String,
    /// Only meaningful for [`SpotKind::IndexFuture`].
    pub(crate) expiry: String,
    pub(crate) kind: SpotKind,
}

/// Everything the subscription planner needs from the master.
#[derive(Clone, Debug, Default)]
pub(crate) struct InstrumentMaster {
    pub(crate) options: Vec<OptionContract>,
    pub(crate) spots: Vec<SpotRow>,
    pub(crate) report: ParseReport,
}

/// What the parse actually saw.
///
/// Kept because "the plan is empty" is otherwise a guessing game: this says how many
/// rows were read, how many became contracts, and which rule rejected the rest.
#[derive(Clone, Debug, Default)]
pub(crate) struct ParseReport {
    pub(crate) total_rows: usize,
    pub(crate) option_rows: usize,
    pub(crate) spot_rows: usize,
    pub(crate) ignored_instrument: usize,
    pub(crate) skipped_bad_security_id: usize,
    pub(crate) skipped_bad_expiry: usize,
    pub(crate) skipped_bad_strike: usize,
    pub(crate) skipped_bad_option_type: usize,
    pub(crate) skipped_unsupported_segment: usize,
    pub(crate) skipped_duplicate: usize,
    pub(crate) missing_columns: Vec<String>,
}

/// Column names this parser needs, each with the aliases Dhan has used.
///
/// The first name is what today's detailed master uses; the rest keep the parser
/// working against the plain master and older revisions.
const COLUMNS: &[(&str, &[&str])] = &[
    ("EXCH_ID", &["EXCHANGE", "EXCH"]),
    ("SEGMENT", &["SEGMENT_NAME"]),
    ("SECURITY_ID", &["SECURITYID"]),
    ("INSTRUMENT", &["INSTRUMENT_NAME"]),
    ("UNDERLYING_SYMBOL", &["UNDERLYING", "SEM_UNDERLYING"]),
    ("SYMBOL_NAME", &["TRADING_SYMBOL", "SEM_TRADING_SYMBOL"]),
    ("SM_EXPIRY_DATE", &["EXPIRY_DATE", "SEM_EXPIRY_DATE"]),
    ("STRIKE_PRICE", &["SEM_STRIKE_PRICE"]),
    ("OPTION_TYPE", &["SEM_OPTION_TYPE"]),
    ("LOT_SIZE", &["SEM_LOT_UNITS"]),
];

/// Resolved column positions for the header we actually got.
struct Header {
    exchange_id: Option<usize>,
    segment: Option<usize>,
    security_id: Option<usize>,
    instrument: Option<usize>,
    underlying_symbol: Option<usize>,
    symbol_name: Option<usize>,
    expiry: Option<usize>,
    strike_price: Option<usize>,
    option_type: Option<usize>,
    lot_size: Option<usize>,
    missing: Vec<String>,
}

/// Read and parse the instrument master at `path`.
pub(crate) fn load_instrument_master(path: impl AsRef<Path>) -> Result<InstrumentMaster> {
    let path = path.as_ref();
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read instrument master: {}", path.display()))?;

    parse_instrument_master(&text)
        .with_context(|| format!("failed to parse instrument master: {}", path.display()))
}

/// Parse the master from memory. Split out from IO so it can be exercised directly.
pub(crate) fn parse_instrument_master(text: &str) -> Result<InstrumentMaster> {
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let header_line = lines.next().context("instrument master is empty")?;
    let header = Header::locate(&split_csv_line(header_line));

    // Without a security id no instrument can be addressed at Dhan, so this one is
    // fatal rather than a skip counter: the result would be a plausible-looking
    // empty universe.
    let security_id_at = header
        .security_id
        .context("instrument master has no SECURITY_ID column")?;

    let mut master = InstrumentMaster {
        report: ParseReport {
            missing_columns: header.missing.clone(),
            ..ParseReport::default()
        },
        ..InstrumentMaster::default()
    };
    // Dedupe on (segment, security id): the pair Dhan addresses an instrument by, so a
    // repeated row cannot produce a duplicate subscription.
    let mut seen = std::collections::HashSet::new();

    for line in lines {
        let cells = split_csv_line(line);
        master.report.total_rows += 1;

        let instrument = header.field(&cells, header.instrument);
        let Some(class) = classify(instrument) else {
            master.report.ignored_instrument += 1;
            continue;
        };

        let security_id = cells.get(security_id_at).map(String::as_str).unwrap_or("");
        if !is_valid_security_id(security_id) {
            master.report.skipped_bad_security_id += 1;
            continue;
        }

        let exchange_id = header.field(&cells, header.exchange_id);
        let segment_code = header.field(&cells, header.segment);
        let underlying_symbol = header.field(&cells, header.underlying_symbol);

        match class {
            Class::Option(kind) => {
                let Ok(segment) = derivative_segment(exchange_id, segment_code) else {
                    master.report.skipped_unsupported_segment += 1;
                    continue;
                };
                if !seen.insert((segment, security_id.to_owned())) {
                    master.report.skipped_duplicate += 1;
                    continue;
                }

                let expiry = header.field(&cells, header.expiry);
                if parse_iso_date(expiry).is_err() {
                    master.report.skipped_bad_expiry += 1;
                    continue;
                }
                let Some(strike_units) = parse_strike(header.field(&cells, header.strike_price))
                else {
                    master.report.skipped_bad_strike += 1;
                    continue;
                };
                let Some(option_type) = parse_option_type(header.field(&cells, header.option_type))
                else {
                    master.report.skipped_bad_option_type += 1;
                    continue;
                };
                if underlying_symbol.is_empty() {
                    master.report.skipped_bad_option_type += 1;
                    continue;
                }

                master.options.push(OptionContract {
                    segment,
                    security_id: security_id.to_owned(),
                    underlying_symbol: underlying_symbol.to_owned(),
                    expiry: expiry.to_owned(),
                    strike_units,
                    option_type,
                    lot_size: parse_lot_size(header.field(&cells, header.lot_size)),
                    kind,
                });
                master.report.option_rows += 1;
            }
            Class::Spot(kind) => {
                let Ok(segment) = spot_segment(exchange_id, segment_code, instrument) else {
                    master.report.skipped_unsupported_segment += 1;
                    continue;
                };
                if !seen.insert((segment, security_id.to_owned())) {
                    master.report.skipped_duplicate += 1;
                    continue;
                }

                master.spots.push(SpotRow {
                    segment,
                    security_id: security_id.to_owned(),
                    exchange_id: exchange_id.to_owned(),
                    underlying_symbol: underlying_symbol.to_owned(),
                    symbol_name: header.field(&cells, header.symbol_name).to_owned(),
                    expiry: header.field(&cells, header.expiry).to_owned(),
                    kind,
                });
                master.report.spot_rows += 1;
            }
        }
    }

    if master.options.is_empty() {
        bail!(
            "instrument master yielded no option contracts from {} rows; the layout has probably changed",
            master.report.total_rows
        );
    }
    Ok(master)
}

/// What a row is useful for.
enum Class {
    Option(ChainKind),
    Spot(SpotKind),
}

/// Classify a row by its `INSTRUMENT` value, per the DhanHQ v2 Annexure.
///
/// `OPTFUT` (options on commodity futures) is deliberately excluded: it is a
/// different product from an equity/index option chain and its underlying is a
/// futures contract, not a spot, so it cannot be centred on a spot price the way this
/// planner works. `OPTCUR`, `FUTSTK`, `FUTCOM` and `FUTCUR` are excluded for the same
/// reason — nothing here needs them.
fn classify(instrument: &str) -> Option<Class> {
    match instrument {
        "OPTIDX" => Some(Class::Option(ChainKind::Index)),
        "OPTSTK" => Some(Class::Option(ChainKind::Stock)),
        "INDEX" => Some(Class::Spot(SpotKind::Index)),
        "EQUITY" => Some(Class::Spot(SpotKind::Equity)),
        "FUTIDX" => Some(Class::Spot(SpotKind::IndexFuture)),
        _ => None,
    }
}

/// A Dhan security id is a positive integer, short enough to fit a `u32` in the feed.
fn is_valid_security_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 10
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<u32>().is_ok_and(|id| id > 0)
}

/// Parse a strike into integer units, rejecting anything not strictly positive.
///
/// A zero strike is what futures rows carry, so it is not a valid option strike.
fn parse_strike(value: &str) -> Option<i64> {
    let price = value.trim().parse::<f64>().ok()?;
    (price.is_finite() && price > 0.0).then(|| to_strike_units(price))
}

fn parse_option_type(value: &str) -> Option<OptionType> {
    match value.trim().to_ascii_uppercase().as_str() {
        "CE" | "CALL" => Some(OptionType::Call),
        "PE" | "PUT" => Some(OptionType::Put),
        _ => None,
    }
}

/// Lot size, or 0 when the master does not give a usable one.
///
/// Not a skip reason: a missing lot size does not stop us subscribing to the
/// contract, it only stops us sizing an order in it.
fn parse_lot_size(value: &str) -> u32 {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|size| size.is_finite() && *size >= 0.0 && size.fract() == 0.0)
        .map_or(0, |size| size as u32)
}

impl Header {
    /// Locate every column this parser needs, recording the ones that are absent.
    fn locate(header: &[String]) -> Self {
        let normalized: Vec<String> = header.iter().map(|name| normalize_column(name)).collect();
        let find = |names: &(&str, &[&str])| -> Option<usize> {
            std::iter::once(names.0)
                .chain(names.1.iter().copied())
                .find_map(|name| {
                    let want = normalize_column(name);
                    normalized.iter().position(|actual| *actual == want)
                })
        };

        let resolved: Vec<Option<usize>> = COLUMNS.iter().map(find).collect();
        let missing = COLUMNS
            .iter()
            .zip(&resolved)
            .filter(|(_, at)| at.is_none())
            .map(|(names, _)| names.0.to_owned())
            .collect();

        Self {
            exchange_id: resolved[0],
            segment: resolved[1],
            security_id: resolved[2],
            instrument: resolved[3],
            underlying_symbol: resolved[4],
            symbol_name: resolved[5],
            expiry: resolved[6],
            strike_price: resolved[7],
            option_type: resolved[8],
            lot_size: resolved[9],
            missing,
        }
    }

    /// Trimmed value of a column, or `""` when the column or cell is absent.
    fn field<'row>(&self, cells: &'row [String], at: Option<usize>) -> &'row str {
        at.and_then(|at| cells.get(at))
            .map(|cell| cell.trim())
            .unwrap_or("")
    }
}

/// Case- and separator-insensitive column name, so `SM_EXPIRY_DATE`, `Sm Expiry Date`
/// and `sm-expiry-date` all match.
fn normalize_column(name: &str) -> String {
    name.trim()
        .to_ascii_uppercase()
        .chars()
        .map(|character| match character {
            ' ' | '-' => '_',
            other => other,
        })
        .collect()
}

/// Split one CSV line, honouring double-quoted fields.
///
/// Today's detailed master contains no quoted fields at all, so a plain `split(',')`
/// would work — but instrument display names are exactly the kind of free text that
/// acquires a comma, and if one ever does, a naive split corrupts every field after
/// it rather than failing visibly. The quote handling is a few lines and removes that
/// whole failure mode.
fn split_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut characters = line.chars().peekable();

    while let Some(character) = characters.next() {
        match character {
            '"' if in_quotes && characters.peek() == Some(&'"') => {
                // A doubled quote inside a quoted field is a literal quote.
                current.push('"');
                characters.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => fields.push(std::mem::take(&mut current)),
            other => current.push(other),
        }
    }
    fields.push(current);
    fields
}
