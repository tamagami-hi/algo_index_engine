use super::*;

use crate::config::sandbox::Sandbox;

struct Edge {
    origin: String,
    engine: EngineState,
    shutdown: CancellationToken,
    server: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl Edge {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let engine = EngineState::new();
        let shutdown = CancellationToken::new();
        let server = tokio::spawn(serve(engine.clone(), listener, shutdown.clone(), None));
        Self {
            origin,
            engine,
            shutdown,
            server,
        }
    }

    fn url(&self) -> String {
        format!("{}{}", self.origin, postback::POSTBACK_PATH)
    }

    async fn stop(self) {
        self.shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(2), self.server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}

fn stored_bodies(day: &str) -> Vec<String> {
    let path = postback::journal_path(day);
    std::fs::read_to_string(&path)
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let record: serde_json::Value = serde_json::from_str(line).expect("journal line");
            record["body"].as_str().expect("body").to_owned()
        })
        .collect()
}

#[tokio::test]
async fn a_broker_postback_is_answered_only_once_it_is_on_disk() {
    let _sandbox = Sandbox::new();
    let day = crate::dhan_api::instruments::ist_today().unwrap();
    let edge = Edge::start().await;
    let body = r#"{"dhanClientId":"1000000009","orderId":"112111182198","orderStatus":"TRADED","filled_qty":"50","legName":"ENTRY_LEG"}"#;

    let response = reqwest::Client::new()
        .post(edge.url())
        .header("content-type", "application/json")
        .header("x-forwarded-for", "203.0.113.7")
        .body(body)
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a webhook reads any non-2xx as a delivery failure, so 200 must mean the record is durable"
    );

    assert_eq!(
        stored_bodies(&day),
        vec![body.to_owned()],
        "the answer is only truthful if the payload really is on disk"
    );

    let snapshot = edge.engine.snapshot();
    assert_eq!(snapshot.postbacks.received, 1);
    assert_eq!(snapshot.postbacks.stored, 1);
    assert_eq!(snapshot.postbacks.oversized, 0);
    assert_eq!(snapshot.postbacks.unwritable, 0);
    assert_eq!(snapshot.postbacks.journal_records, 1);
    assert_eq!(
        snapshot.postbacks.journal_day.as_deref(),
        Some(day.as_str())
    );
    assert_eq!(snapshot.postbacks.bytes, body.len() as u64);
    assert!(snapshot.postbacks.last_received_at_ms.unwrap_or(0) > 0);

    edge.stop().await;
}

#[tokio::test]
async fn the_sender_is_recorded_without_being_believed() {
    let _sandbox = Sandbox::new();
    let day = crate::dhan_api::instruments::ist_today().unwrap();
    let edge = Edge::start().await;
    let forged = "x".repeat(4_000);

    let response = reqwest::Client::new()
        .post(edge.url())
        .header("x-forwarded-for", &forged)
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let path = postback::journal_path(&day);
    let line = std::fs::read_to_string(&path).unwrap();
    let record: serde_json::Value = serde_json::from_str(line.trim()).expect("journal line");
    let source = record["source"].as_str().expect("source");
    assert!(
        source.len() < forged.len(),
        "an unauthenticated header must not be able to inflate the journal, got {} chars",
        source.len()
    );
    assert!(source.chars().all(|character| character == 'x'));

    edge.stop().await;
}

#[tokio::test]
async fn an_oversized_body_is_refused_and_counted_rather_than_stored() {
    let _sandbox = Sandbox::new();
    let day = crate::dhan_api::instruments::ist_today().unwrap();
    let edge = Edge::start().await;
    let body = "0".repeat(postback::MAX_POSTBACK_BYTES + 1);

    let response = reqwest::Client::new()
        .post(edge.url())
        .body(body)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(
        stored_bodies(&day).is_empty(),
        "nothing oversized reaches the journal"
    );

    let snapshot = edge.engine.snapshot();
    assert_eq!(snapshot.postbacks.received, 1);
    assert_eq!(snapshot.postbacks.stored, 0);
    assert_eq!(
        snapshot.postbacks.oversized, 1,
        "a refusal has to be visible to the operator, not only in the log"
    );

    edge.stop().await;
}

#[tokio::test]
async fn a_body_past_the_transport_limit_never_reaches_the_handler() {
    let _sandbox = Sandbox::new();
    let day = crate::dhan_api::instruments::ist_today().unwrap();
    let edge = Edge::start().await;
    let body = "0".repeat(postback::POSTBACK_BODY_LIMIT_BYTES + 1);

    let response = reqwest::Client::new()
        .post(edge.url())
        .body(body)
        .send()
        .await
        .unwrap();

    assert!(
        !response.status().is_success(),
        "an unauthenticated endpoint must refuse to buffer past its own limit"
    );
    assert!(stored_bodies(&day).is_empty());

    edge.stop().await;
}

#[tokio::test]
async fn a_payload_the_engine_cannot_yet_interpret_is_still_kept() {
    let _sandbox = Sandbox::new();
    let day = crate::dhan_api::instruments::ist_today().unwrap();
    let edge = Edge::start().await;
    let client = reqwest::Client::new();
    let bodies = [
        "{\"orderStatus\":\"TRANSIT\"}",
        "{\"legName\": ,}",
        "not json at all",
        "",
    ];

    for body in bodies {
        let response = client.post(edge.url()).body(body).send().await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "parsing is not this endpoint's job, so a payload it cannot read is not a failure"
        );
    }

    assert_eq!(
        stored_bodies(&day),
        bodies
            .iter()
            .map(|body| (*body).to_owned())
            .collect::<Vec<String>>(),
        "every postback is kept verbatim and in order, malformed or not"
    );

    let snapshot = edge.engine.snapshot();
    assert_eq!(snapshot.postbacks.stored, bodies.len() as u64);
    assert_eq!(snapshot.postbacks.journal_records, bodies.len());

    edge.stop().await;
}

#[tokio::test]
async fn only_a_post_is_accepted() {
    let _sandbox = Sandbox::new();
    let edge = Edge::start().await;

    let response = reqwest::Client::new().get(edge.url()).send().await.unwrap();

    assert_eq!(
        response.status(),
        StatusCode::METHOD_NOT_ALLOWED,
        "the broker posts; a readable journal endpoint would publish order activity unauthenticated"
    );

    edge.stop().await;
}
