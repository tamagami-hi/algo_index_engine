use anyhow::{Result, bail};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum ExchangeSegment {
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

}

pub(crate) fn derivative_segment(exchange_id: &str, segment: &str) -> Result<ExchangeSegment> {
    match (exchange_id, segment) {
        ("NSE", "D") => Ok(ExchangeSegment::NseFno),
        ("BSE", "D") => Ok(ExchangeSegment::BseFno),
        ("MCX", "M") => Ok(ExchangeSegment::McxComm),
        _ => bail!("unsupported option exchange/segment: {exchange_id}/{segment}"),
    }
}

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

pub(crate) fn exchange_id_of(segment: ExchangeSegment) -> Result<&'static str> {
    match segment {
        ExchangeSegment::NseFno => Ok("NSE"),
        ExchangeSegment::BseFno => Ok("BSE"),
        ExchangeSegment::McxComm => Ok("MCX"),
        _ => bail!("{} is not an option segment", segment.as_str()),
    }
}
