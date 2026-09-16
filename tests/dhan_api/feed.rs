use super::*;
use crate::dhan_api::instruments::ExchangeSegment;

const IDX_I: u8 = 0;
const NSE_EQ: u8 = 1;
const NSE_FNO: u8 = 2;

fn header(code: u8, len: u16, segment: u8, security_id: i32) -> Vec<u8> {
    let mut bytes = vec![code];
    bytes.extend_from_slice(&len.to_le_bytes());
    bytes.push(segment);
    bytes.extend_from_slice(&security_id.to_le_bytes());
    bytes
}

#[test]
fn the_documented_segment_bytes_map_to_the_right_segments() {
    assert_eq!(ExchangeSegment::from_code(IDX_I), Some(ExchangeSegment::IdxI));
    assert_eq!(ExchangeSegment::from_code(NSE_EQ), Some(ExchangeSegment::NseEq));
    assert_eq!(ExchangeSegment::from_code(NSE_FNO), Some(ExchangeSegment::NseFno));
    assert_eq!(ExchangeSegment::from_code(4), Some(ExchangeSegment::BseEq));
    assert_eq!(ExchangeSegment::from_code(5), Some(ExchangeSegment::McxComm));
    assert_eq!(ExchangeSegment::from_code(8), Some(ExchangeSegment::BseFno));
    assert_eq!(ExchangeSegment::from_code(6), None, "6 is not assigned");
    assert_eq!(ExchangeSegment::from_code(99), None);
}

fn full_packet(security_id: i32, ltp: f32, bid: f32, ask: f32) -> Vec<u8> {
    let mut bytes = header(CODE_FULL, 162, NSE_FNO, security_id);
    bytes.extend_from_slice(&ltp.to_le_bytes());
    bytes.extend_from_slice(&7i16.to_le_bytes());
    bytes.extend_from_slice(&1_700_000_000i32.to_le_bytes());
    bytes.extend_from_slice(&101.5f32.to_le_bytes());
    bytes.extend_from_slice(&123_456i32.to_le_bytes());
    bytes.extend_from_slice(&500i32.to_le_bytes());
    bytes.extend_from_slice(&600i32.to_le_bytes());
    bytes.extend_from_slice(&987_654i32.to_le_bytes());
    bytes.extend_from_slice(&1_000_000i32.to_le_bytes());
    bytes.extend_from_slice(&900_000i32.to_le_bytes());
    bytes.extend_from_slice(&100.0f32.to_le_bytes());
    bytes.extend_from_slice(&0.0f32.to_le_bytes());
    bytes.extend_from_slice(&110.0f32.to_le_bytes());
    bytes.extend_from_slice(&95.0f32.to_le_bytes());
    for level in 0..5i32 {
        bytes.extend_from_slice(&(10 + level).to_le_bytes());
        bytes.extend_from_slice(&(20 + level).to_le_bytes());
        bytes.extend_from_slice(&(1 + level as i16).to_le_bytes());
        bytes.extend_from_slice(&(2 + level as i16).to_le_bytes());
        bytes.extend_from_slice(&(bid - level as f32).to_le_bytes());
        bytes.extend_from_slice(&(ask + level as f32).to_le_bytes());
    }
    assert_eq!(bytes.len(), 162, "full packet must be exactly 162 bytes");
    bytes
}

#[test]
fn full_packet_decodes_top_of_book_and_open_interest() {
    let frame = full_packet(43041, 134.25, 133.55, 134.00);

    let messages = decode_frame(&frame);
    assert_eq!(messages.len(), 1);

    let message = messages[0];
    assert_eq!(message.header.code, CODE_FULL);
    assert_eq!(message.header.segment, Some(ExchangeSegment::NseFno));
    assert_eq!(message.header.security_id, 43041);

    let Packet::Full(full) = message.packet else {
        panic!("expected a full packet, got {:?}", message.packet);
    };

    assert_eq!(full.last_price, 134.25);
    assert_eq!(full.volume, 123_456);
    assert_eq!(full.open_interest, 987_654);
    assert_eq!(full.open_interest_day_high, 1_000_000);
    assert_eq!(full.open_interest_day_low, 900_000);
    assert_eq!(full.day_high, 110.0);
    assert_eq!(full.day_low, 95.0);
    assert_eq!(full.total_sell_quantity, 500);
    assert_eq!(full.total_buy_quantity, 600);

    assert_eq!(full.best_bid(), Some((133.55, 10)));
    assert_eq!(full.best_ask(), Some((134.00, 20)));
    assert_eq!(full.depth[4].bid_price, 133.55 - 4.0);
    assert_eq!(full.depth[4].ask_price, 134.00 + 4.0);
}

#[test]
fn a_single_frame_carrying_many_packets_is_split_by_declared_length() {
    let mut frame = Vec::new();
    frame.extend_from_slice(&full_packet(1001, 10.0, 9.5, 10.5));

    let mut ticker = header(CODE_TICKER, 16, IDX_I, 13);
    ticker.extend_from_slice(&25_642.8f32.to_le_bytes());
    ticker.extend_from_slice(&1_700_000_001i32.to_le_bytes());
    frame.extend_from_slice(&ticker);

    let mut oi = header(CODE_OI, 12, NSE_FNO, 1001);
    oi.extend_from_slice(&555i32.to_le_bytes());
    frame.extend_from_slice(&oi);

    let mut index = header(CODE_INDEX, 16, IDX_I, 21);
    index.extend_from_slice(&11.75f32.to_le_bytes());
    index.extend_from_slice(&0i32.to_le_bytes());
    frame.extend_from_slice(&index);

    let messages = decode_frame(&frame);

    assert_eq!(
        messages.len(),
        4,
        "every packet in the frame must be recovered"
    );
    assert!(matches!(messages[0].packet, Packet::Full(_)));
    assert_eq!(messages[1].header.security_id, 13);
    assert!(
        matches!(messages[1].packet, Packet::Ticker { last_price, .. } if last_price == 25_642.8)
    );
    assert_eq!(
        messages[2].packet,
        Packet::OpenInterest {
            open_interest: 555
        }
    );
    assert_eq!(messages[3].header.security_id, 21);
    assert_eq!(messages[3].packet, Packet::Index { last_price: 11.75 });
}

#[test]
fn a_truncated_trailing_packet_is_dropped_rather_than_read_past_the_end() {
    let mut frame = full_packet(2002, 50.0, 49.0, 51.0);
    frame.extend_from_slice(&header(CODE_FULL, 162, NSE_FNO, 3003));
    frame.extend_from_slice(&1.0f32.to_le_bytes());

    let messages = decode_frame(&frame);

    assert_eq!(messages.len(), 1, "the partial second packet must be ignored");
    assert_eq!(messages[0].header.security_id, 2002);
}

#[test]
fn an_unknown_code_advances_by_its_declared_length_without_losing_the_next_packet() {
    let mut frame = header(99, 12, NSE_EQ, 4004);
    frame.extend_from_slice(&0i32.to_le_bytes());

    let mut oi = header(CODE_OI, 12, NSE_FNO, 5005);
    oi.extend_from_slice(&42i32.to_le_bytes());
    frame.extend_from_slice(&oi);

    let messages = decode_frame(&frame);

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].packet, Packet::Unknown { code: 99, len: 12 });
    assert_eq!(
        messages[1].packet,
        Packet::OpenInterest { open_interest: 42 }
    );
}

#[test]
fn a_zero_bid_is_reported_as_no_bid_rather_than_a_zero_price() {
    let frame = full_packet(6006, 20.0, 0.0, 0.0);
    let messages = decode_frame(&frame);

    let Packet::Full(full) = messages[0].packet else {
        panic!("expected a full packet");
    };
    assert_eq!(full.best_bid(), None, "a zero price is not a tradable bid");
    assert_eq!(full.best_ask(), None, "a zero price is not a tradable ask");
}

#[test]
fn a_frame_shorter_than_a_header_yields_nothing() {
    assert!(decode_frame(&[]).is_empty());
    assert!(decode_frame(&[8, 0, 0]).is_empty());
}

#[test]
fn disconnect_carries_its_reason_code() {
    let mut frame = header(CODE_DISCONNECT, 10, IDX_I, 0);
    frame.extend_from_slice(&805i16.to_le_bytes());

    let messages = decode_frame(&frame);
    assert_eq!(messages[0].packet, Packet::Disconnect { reason: 805 });
}
