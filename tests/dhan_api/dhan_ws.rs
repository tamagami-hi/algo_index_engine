use super::*;
use crate::dhan_api::instruments::ExchangeSegment;
use crate::dhan_api::instruments::subscription::MAX_PER_MESSAGE;

fn subscription(security_id: usize) -> Subscription {
    Subscription {
        segment: ExchangeSegment::NseFno,
        security_id: security_id.to_string(),
    }
}

#[test]
fn subscription_requests_full_mode_and_names_segments_as_dhan_expects() {
    let batch = vec![
        subscription(43041),
        Subscription {
            segment: ExchangeSegment::IdxI,
            security_id: "21".to_owned(),
        },
    ];

    let payload: serde_json::Value =
        serde_json::from_str(&subscribe_message(&batch)).expect("subscribe message must be JSON");

    assert_eq!(
        payload["RequestCode"], 21,
        "21 is Subscribe-Full; 15 (ticker) and 17 (quote) carry no bid/ask"
    );
    assert_eq!(payload["InstrumentCount"], 2);
    assert_eq!(payload["InstrumentList"][0]["ExchangeSegment"], "NSE_FNO");
    assert_eq!(
        payload["InstrumentList"][0]["SecurityId"], "43041",
        "SecurityId must be a JSON string, not a number"
    );
    assert_eq!(payload["InstrumentList"][1]["ExchangeSegment"], "IDX_I");
    assert_eq!(payload["InstrumentList"][1]["SecurityId"], "21");
}

#[test]
fn no_single_message_exceeds_the_hundred_instrument_limit() {
    let pool = Pool {
        label: "index_options",
        instruments: (1..=250).map(subscription).collect(),
    };

    let batches: Vec<&[Subscription]> = pool.messages().collect();
    assert_eq!(batches.len(), 3);

    let mut total = 0;
    for batch in &batches {
        assert!(batch.len() <= MAX_PER_MESSAGE);
        let payload: serde_json::Value = serde_json::from_str(&subscribe_message(batch)).unwrap();
        assert_eq!(payload["InstrumentCount"], batch.len());
        assert_eq!(
            payload["InstrumentList"].as_array().unwrap().len(),
            batch.len(),
            "InstrumentCount must match the list it describes"
        );
        total += batch.len();
    }
    assert_eq!(total, 250, "batching must not drop an instrument");
}
