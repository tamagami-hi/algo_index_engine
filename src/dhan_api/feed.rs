use super::instruments::ExchangeSegment;

pub(crate) const HEADER_BYTES: usize = 8;

pub(crate) const CODE_INDEX: u8 = 1;
pub(crate) const CODE_TICKER: u8 = 2;
pub(crate) const CODE_QUOTE: u8 = 4;
pub(crate) const CODE_OI: u8 = 5;
pub(crate) const CODE_PREV_CLOSE: u8 = 6;
pub(crate) const CODE_FULL: u8 = 8;
pub(crate) const CODE_DISCONNECT: u8 = 50;

const DEPTH_LEVELS: usize = 5;
const DEPTH_LEVEL_BYTES: usize = 20;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Header {
    pub(crate) code: u8,
    pub(crate) declared_len: usize,
    pub(crate) segment: Option<ExchangeSegment>,
    pub(crate) security_id: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct DepthLevel {
    pub(crate) bid_quantity: i32,
    pub(crate) ask_quantity: i32,
    pub(crate) bid_orders: i16,
    pub(crate) ask_orders: i16,
    pub(crate) bid_price: f32,
    pub(crate) ask_price: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Full {
    pub(crate) last_price: f32,
    pub(crate) last_quantity: i16,
    pub(crate) last_trade_time: i32,
    pub(crate) average_price: f32,
    pub(crate) volume: i32,
    pub(crate) total_sell_quantity: i32,
    pub(crate) total_buy_quantity: i32,
    pub(crate) open_interest: i32,
    pub(crate) open_interest_day_high: i32,
    pub(crate) open_interest_day_low: i32,
    pub(crate) day_open: f32,
    pub(crate) day_close: f32,
    pub(crate) day_high: f32,
    pub(crate) day_low: f32,
    pub(crate) depth: [DepthLevel; DEPTH_LEVELS],
}

impl Full {
    pub(crate) fn best_bid(&self) -> Option<(f32, i32)> {
        let level = self.depth.first()?;
        (level.bid_price > 0.0).then_some((level.bid_price, level.bid_quantity))
    }

    pub(crate) fn best_ask(&self) -> Option<(f32, i32)> {
        let level = self.depth.first()?;
        (level.ask_price > 0.0).then_some((level.ask_price, level.ask_quantity))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Packet {
    Index { last_price: f32 },
    Ticker { last_price: f32, last_trade_time: i32 },
    Quote { last_price: f32, volume: i32 },
    OpenInterest { open_interest: i32 },
    PrevClose { close: f32, open_interest: i32 },
    Full(Full),
    Disconnect { reason: i16 },
    Unknown { code: u8, len: usize },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Message {
    pub(crate) header: Header,
    pub(crate) packet: Packet,
}

fn u8_at(bytes: &[u8], at: usize) -> Option<u8> {
    bytes.get(at).copied()
}

fn i16_at(bytes: &[u8], at: usize) -> Option<i16> {
    Some(i16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn i32_at(bytes: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn f32_at(bytes: &[u8], at: usize) -> Option<f32> {
    Some(f32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

pub(crate) fn read_header(bytes: &[u8]) -> Option<Header> {
    Some(Header {
        code: u8_at(bytes, 0)?,
        declared_len: i16_at(bytes, 1)?.max(0) as usize,
        segment: ExchangeSegment::from_code(u8_at(bytes, 3)?),
        security_id: i32_at(bytes, 4)?,
    })
}

const fn known_len(code: u8) -> Option<usize> {
    match code {
        CODE_TICKER => Some(16),
        CODE_QUOTE => Some(50),
        CODE_OI => Some(12),
        CODE_PREV_CLOSE => Some(16),
        CODE_FULL => Some(162),
        CODE_DISCONNECT => Some(10),
        _ => None,
    }
}

fn read_depth(bytes: &[u8], at: usize) -> [DepthLevel; DEPTH_LEVELS] {
    let mut depth = [DepthLevel::default(); DEPTH_LEVELS];
    for (level, slot) in depth.iter_mut().enumerate() {
        let base = at + level * DEPTH_LEVEL_BYTES;
        *slot = DepthLevel {
            bid_quantity: i32_at(bytes, base).unwrap_or(0),
            ask_quantity: i32_at(bytes, base + 4).unwrap_or(0),
            bid_orders: i16_at(bytes, base + 8).unwrap_or(0),
            ask_orders: i16_at(bytes, base + 10).unwrap_or(0),
            bid_price: f32_at(bytes, base + 12).unwrap_or(0.0),
            ask_price: f32_at(bytes, base + 16).unwrap_or(0.0),
        };
    }
    depth
}

fn read_packet(code: u8, bytes: &[u8], len: usize) -> Packet {
    match code {
        CODE_INDEX => Packet::Index {
            last_price: f32_at(bytes, 8).unwrap_or(0.0),
        },
        CODE_TICKER => Packet::Ticker {
            last_price: f32_at(bytes, 8).unwrap_or(0.0),
            last_trade_time: i32_at(bytes, 12).unwrap_or(0),
        },
        CODE_QUOTE => Packet::Quote {
            last_price: f32_at(bytes, 8).unwrap_or(0.0),
            volume: i32_at(bytes, 22).unwrap_or(0),
        },
        CODE_OI => Packet::OpenInterest {
            open_interest: i32_at(bytes, 8).unwrap_or(0),
        },
        CODE_PREV_CLOSE => Packet::PrevClose {
            close: f32_at(bytes, 8).unwrap_or(0.0),
            open_interest: i32_at(bytes, 12).unwrap_or(0),
        },
        CODE_FULL => Packet::Full(Full {
            last_price: f32_at(bytes, 8).unwrap_or(0.0),
            last_quantity: i16_at(bytes, 12).unwrap_or(0),
            last_trade_time: i32_at(bytes, 14).unwrap_or(0),
            average_price: f32_at(bytes, 18).unwrap_or(0.0),
            volume: i32_at(bytes, 22).unwrap_or(0),
            total_sell_quantity: i32_at(bytes, 26).unwrap_or(0),
            total_buy_quantity: i32_at(bytes, 30).unwrap_or(0),
            open_interest: i32_at(bytes, 34).unwrap_or(0),
            open_interest_day_high: i32_at(bytes, 38).unwrap_or(0),
            open_interest_day_low: i32_at(bytes, 42).unwrap_or(0),
            day_open: f32_at(bytes, 46).unwrap_or(0.0),
            day_close: f32_at(bytes, 50).unwrap_or(0.0),
            day_high: f32_at(bytes, 54).unwrap_or(0.0),
            day_low: f32_at(bytes, 58).unwrap_or(0.0),
            depth: read_depth(bytes, 62),
        }),
        CODE_DISCONNECT => Packet::Disconnect {
            reason: i16_at(bytes, 8).unwrap_or(0),
        },
        other => Packet::Unknown { code: other, len },
    }
}

pub(crate) fn decode_frame(frame: &[u8]) -> Vec<Message> {
    let mut messages = Vec::new();
    let mut at = 0;

    while at + HEADER_BYTES <= frame.len() {
        let rest = &frame[at..];
        let Some(header) = read_header(rest) else {
            break;
        };

        let len = match known_len(header.code) {
            Some(len) => len,
            None if header.declared_len >= HEADER_BYTES => header.declared_len,
            None => break,
        };
        if len > rest.len() {
            break;
        }

        messages.push(Message {
            header,
            packet: read_packet(header.code, rest, len),
        });
        at += len;
    }

    messages
}

#[cfg(test)]
#[path = "../../tests/dhan_api/feed.rs"]
mod tests;
