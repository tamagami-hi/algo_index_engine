//! Dhan exchange segments.
//!
//! Dhan addresses an instrument as a `(ExchangeSegment, SecurityId)` pair and uses
//! two different spellings for the segment half:
//!
//! - a **name** in REST bodies and in the WebSocket subscribe JSON (`"NSE_FNO"`),
//! - a **numeric code** in byte 3 of every binary feed packet (`2`).
//!
//! Both come from the DhanHQ v2 Annexure. They are defined side by side here because
//! a wrong segment does not error, it silently addresses a different instrument (or
//! none at all), so this table is the one place to get it right.

use anyhow::{Result, bail};

/// A Dhan exchange segment, as named in REST payloads and subscribe messages.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum ExchangeSegment {
    /// Index values. Dhan groups every index here regardless of which exchange
    /// computes it, so NIFTY and SENSEX both live in `IDX_I`.
    IdxI,
    NseEq,
    NseFno,
    NseCurrency,
    BseEq,
    BseFno,
    BseCurrency,
    McxComm,
}

impl ExchangeSegment {
    /// The segment name Dhan expects in `ExchangeSegment` fields.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::IdxI => "IDX_I",
            Self::NseEq => "NSE_EQ",
            Self::NseFno => "NSE_FNO",
            Self::NseCurrency => "NSE_CURRENCY",
            Self::BseEq => "BSE_EQ",
            Self::BseFno => "BSE_FNO",
            Self::BseCurrency => "BSE_CURRENCY",
            Self::McxComm => "MCX_COMM",
        }
    }

    /// The numeric code carried in byte 3 of a binary feed packet header.
    ///
    /// Note the gap: 6 is unassigned, so this is a lookup table and not an ordinal.
    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::IdxI => 0,
            Self::NseEq => 1,
            Self::NseFno => 2,
            Self::NseCurrency => 3,
            Self::BseEq => 4,
            Self::McxComm => 5,
            Self::BseCurrency => 7,
            Self::BseFno => 8,
        }
    }

    /// Recover a segment from a binary feed packet header byte.
    pub(crate) const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::IdxI),
            1 => Some(Self::NseEq),
            2 => Some(Self::NseFno),
            3 => Some(Self::NseCurrency),
            4 => Some(Self::BseEq),
            5 => Some(Self::McxComm),
            7 => Some(Self::BseCurrency),
            8 => Some(Self::BseFno),
            _ => None,
        }
    }

    /// True for the two segments that carry equity/index option contracts we trade.
    pub(crate) const fn is_derivative(self) -> bool {
        matches!(self, Self::NseFno | Self::BseFno | Self::McxComm)
    }
}

/// The segment an option contract trades in, from the master's `EXCH_ID` + `SEGMENT`.
///
/// `SEGMENT` is a one-letter code in the Dhan master (`D` derivatives, `E` equity,
/// `I` index, `M` commodity, `C` currency), so it is combined with `EXCH_ID` rather
/// than trusted alone. Anything unrecognised is an error, never a guess.
pub(crate) fn derivative_segment(exchange_id: &str, segment: &str) -> Result<ExchangeSegment> {
    match (exchange_id, segment) {
        ("NSE", "D") => Ok(ExchangeSegment::NseFno),
        ("BSE", "D") => Ok(ExchangeSegment::BseFno),
        ("MCX", "M") => Ok(ExchangeSegment::McxComm),
        _ => bail!("unsupported option exchange/segment: {exchange_id}/{segment}"),
    }
}

/// The segment a spot / futures instrument ticks in.
///
/// Indices are checked first: an index row carries `SEGMENT = I` on whichever
/// exchange publishes it, and Dhan feeds all of them under `IDX_I`.
pub(crate) fn spot_segment(
    exchange_id: &str,
    segment: &str,
    instrument: &str,
) -> Result<ExchangeSegment> {
    match (exchange_id, segment, instrument) {
        (_, "I", "INDEX") => Ok(ExchangeSegment::IdxI),
        ("NSE", "E", "EQUITY") => Ok(ExchangeSegment::NseEq),
        ("BSE", "E", "EQUITY") => Ok(ExchangeSegment::BseEq),
        ("NSE", "D", "FUTIDX") => Ok(ExchangeSegment::NseFno),
        ("BSE", "D", "FUTIDX") => Ok(ExchangeSegment::BseFno),
        ("MCX", "M", "FUTIDX") => Ok(ExchangeSegment::McxComm),
        _ => bail!("unsupported spot exchange/segment/instrument: {exchange_id}/{segment}/{instrument}"),
    }
}

/// The exchange id that pairs with an option segment, for matching a spot row.
pub(crate) fn exchange_id_of(segment: ExchangeSegment) -> Result<&'static str> {
    match segment {
        ExchangeSegment::NseFno => Ok("NSE"),
        ExchangeSegment::BseFno => Ok("BSE"),
        ExchangeSegment::McxComm => Ok("MCX"),
        _ => bail!("{} is not an option segment", segment.as_str()),
    }
}
