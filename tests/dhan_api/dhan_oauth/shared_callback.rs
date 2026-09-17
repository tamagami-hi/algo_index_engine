use super::*;
use axum::routing::get;
use tokio::net::TcpListener;

async fn server(callback: &SharedCallback) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = callback.router().route("/health", get(|| async { "ok" }));
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (base, task)
}

#[tokio::test]
async fn serves_backend_and_exactly_one_callback_on_same_listener() {
    let callback = SharedCallback::new("http://127.0.0.1:8787/dhan/callback").unwrap();
    let (base, server) = server(&callback).await;
    let client = reqwest::Client::new();
    assert_eq!(
        client
            .get(format!("{base}/health"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "ok"
    );
    assert_eq!(
        client
            .get(format!("{base}/dhan/callback?tokenId=valid-token"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    let pending = callback.begin().unwrap();
    assert!(callback.begin().is_err());
    for query in [
        "",
        "tokenId=",
        "tokenId=a&tokenId=b",
        "tokenId=a&extra=b",
        "tokenId=%0A",
    ] {
        assert_eq!(
            client
                .get(format!("{base}/dhan/callback?{query}"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        client
            .head(format!("{base}/dhan/callback?tokenId=valid-token"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    let response = client
        .get(format!("{base}/dhan/callback/?tokenId=valid-token"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert!(!response.text().await.unwrap().contains("valid-token"));
    assert_eq!(
        client
            .get(format!("{base}/dhan/callback?tokenId=other-token"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(pending.receive().await.unwrap(), "valid-token");
    assert_eq!(
        client
            .get(format!("{base}/dhan/callback?tokenId=other-token"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    server.abort();
}

#[tokio::test]
async fn cancellation_and_timeout_allow_a_new_attempt() {
    let callback = SharedCallback::new("http://127.0.0.1:8787/dhan/callback").unwrap();
    drop(callback.begin().unwrap());
    let pending = callback.begin().unwrap();
    let task = tokio::spawn(pending.receive());
    task.abort();
    let _ = task.await;
    let pending = callback.begin().unwrap();
    assert!(
        pending
            .receive_with_timeout(Duration::from_millis(1))
            .await
            .is_err()
    );
    assert!(callback.begin().is_ok());
}

#[test]
fn validates_redirect_without_exposing_input() {
    for redirect in [
        "http://127.0.0.1:8787/dhan/callback",
        "http://[::1]:8787/dhan/callback",
        "http://127.0.0.1:8787/dhan/callback?state=abc",
    ] {
        assert!(SharedCallback::new(redirect).is_ok(), "{redirect}");
    }
    for redirect in [
        "garbage",
        "http://example.com/dhan/callback",
        "http://localhost:8787/dhan/callback",
        "https://example.com:8787/dhan/callback",
        "https://user:secret@example.com/dhan/callback",
        "https://example.com:8787/dhan/callback#secret",
        "https://example.com:8787/other",
        "https://example.com:8787/dhan/callback?tokenId=secret",
        "ftp://example.com:8787/dhan/callback",
        "https://example.com:0/dhan/callback",
        "https://example.com/dhan/callback",
        "http://127.0.0.1/dhan/callback",
    ] {
        let error = SharedCallback::new(redirect).err().expect(redirect);
        assert!(!error.to_string().contains("secret"));
    }
}

#[test]
fn requires_the_callback_and_listener_to_use_the_same_port() {
    let callback = SharedCallback::new("http://127.0.0.1:8787/dhan/callback").unwrap();
    assert!(
        callback
            .matches_listener("127.0.0.1:8787".parse().unwrap())
            .is_ok()
    );
    assert!(
        callback
            .matches_listener("127.0.0.1:8788".parse().unwrap())
            .is_err()
    );
}

#[tokio::test]
async fn instances_are_isolated_and_configured_query_is_required() {
    let first = SharedCallback::new("http://127.0.0.1:8787/dhan/callback?state=abc").unwrap();
    let second = SharedCallback::new("http://127.0.0.1:8787/dhan/callback").unwrap();
    let pending = first.begin().unwrap();
    let other = second.begin().unwrap();
    let (base, server) = server(&first).await;
    let client = reqwest::Client::new();
    assert_eq!(
        client
            .get(format!("{base}/dhan/callback?tokenId=abc"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        client
            .get(format!("{base}/dhan/callback?tokenId=abc&state=abc"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(pending.receive().await.unwrap(), "abc");
    assert!(second.begin().is_err());
    drop(other);
    server.abort();
}

#[tokio::test]
async fn concurrent_callbacks_deliver_only_one_token() {
    let callback = SharedCallback::new("http://127.0.0.1:8787/dhan/callback").unwrap();
    let pending = callback.begin().unwrap();
    let (base, server) = server(&callback).await;
    let client = reqwest::Client::new();
    let (first, second) = tokio::join!(
        client
            .get(format!("{base}/dhan/callback?tokenId=first"))
            .send(),
        client
            .get(format!("{base}/dhan/callback?tokenId=second"))
            .send(),
    );
    let statuses = (first.unwrap().status(), second.unwrap().status());
    let expected = match statuses {
        (StatusCode::OK, StatusCode::CONFLICT) => "first",
        (StatusCode::CONFLICT, StatusCode::OK) => "second",
        unexpected => panic!("Unexpected callback statuses: {unexpected:?}"),
    };
    assert_eq!(pending.receive().await.unwrap(), expected);
    server.abort();
}
