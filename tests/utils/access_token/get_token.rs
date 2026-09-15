use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

async fn mock(status: &str, body: &str) -> (String, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/token", listener.local_addr().unwrap());
    let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let request = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = vec![0; 8192];
        let size = stream.read(&mut bytes).await.unwrap();
        stream.write_all(reply.as_bytes()).await.unwrap();
        String::from_utf8(bytes[..size].to_vec()).unwrap()
    });
    (url, request)
}

#[tokio::test]
async fn token_route_sends_passcode_header_and_accepts_cal_response() {
    let (url, request) = mock(
        "200 OK",
        r#"{"access_token":"token","expires_at":4070908800000,"client_id":"123"}"#,
    )
    .await;
    let fetched = fetch_token_with_config(&url, "private-passcode", Some("123"))
        .await
        .unwrap();
    assert_eq!(fetched.token, "token");
    let request = request.await.unwrap();
    assert!(request.starts_with("GET /token HTTP/1.1\r\n"));
    assert!(request.contains("x-token-passcode: private-passcode\r\n"));
}

#[tokio::test]
async fn token_route_rejects_status_errors_without_disclosing_secrets() {
    for status in ["403 Forbidden", "409 Conflict", "302 Found"] {
        let (url, request) = mock(status, r#"{"error":"sensitive-server-secret"}"#).await;
        let error = fetch_token_with_config(&url, "private-passcode", None)
            .await
            .err()
            .unwrap();
        let message = format!("{error:#}");
        assert!(message.contains(&status[..3]));
        assert!(!message.contains("sensitive-server-secret"));
        assert!(!message.contains("private-passcode"));
        request.await.unwrap();
    }
}

#[tokio::test]
async fn token_route_rejects_unsafe_urls_and_blank_passcodes_before_network() {
    for url in [
        "http://example.com/token",
        "https://user:secret@example.com/token",
        "https://example.com/token#secret",
        "https://example.com/token?passcode=secret",
        "https://example.com/token?TOKEN_PASSCODE=secret",
    ] {
        assert!(
            fetch_token_with_config(url, "passcode", None)
                .await
                .is_err()
        );
    }
    for passcode in ["", " ", "bad\nsecret"] {
        assert!(
            fetch_token_with_config("https://example.com/token", passcode, None)
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn token_route_rejects_corrupt_large_and_wrong_account_responses() {
    for body in [
        "sensitive-not-json".to_owned(),
        r#"{"access_token":{"sensitive":"value"}}"#.to_owned(),
        r#"{"access_token":"token","client_id":"other"}"#.to_owned(),
        r#"{"access_token":"bad\nvalue"}"#.to_owned(),
        "x".repeat(64 * 1024 + 1),
    ] {
        let (url, request) = mock("200 OK", &body).await;
        let error = fetch_token_with_config(&url, "passcode", Some("123"))
            .await
            .err()
            .unwrap();
        assert!(!format!("{error:#}").contains("sensitive"));
        request.await.unwrap();
    }
}
