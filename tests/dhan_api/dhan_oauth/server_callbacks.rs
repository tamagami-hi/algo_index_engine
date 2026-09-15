use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

async fn mock_response(status: &str, body: &str) -> (String, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buffer = vec![0; 8192];
        let size = stream.read(&mut buffer).await.unwrap();
        stream.write_all(reply.as_bytes()).await.unwrap();
        String::from_utf8(buffer[..size].to_vec()).unwrap()
    });
    (format!("http://{address}"), task)
}

#[tokio::test]
async fn generates_consent_with_documented_post_and_headers() {
    let (base, request) = mock_response(
        "200 OK",
        r#"{"consentAppId":"id&value","status":"success","consentAppStatus":"GENERATED"}"#,
    )
    .await;
    let config = Config::new("123", "key", "secret").unwrap();
    let url = generate_consent(&config, &base).await.unwrap();
    assert_eq!(
        url,
        "https://auth.dhan.co/login/consentApp-login?consentAppId=id%26value"
    );
    let request = request.await.unwrap();
    assert!(request.starts_with("POST /app/generate-consent?client_id=123 "));
    assert!(request.contains("app_id: key\r\n"));
    assert!(request.contains("app_secret: secret\r\n"));
}

#[tokio::test]
async fn exchanges_callback_with_documented_get_and_encoded_token() {
    let (base, request) = mock_response(
        "200 OK",
        r#"{"dhanClientId":"123","accessToken":"token","expiryTime":"2099-01-01T00:00:00"}"#,
    )
    .await;
    let config = Config::new("123", "key", "secret").unwrap();
    let session = exchange_token(&config, "value&extra", &base).await.unwrap();
    assert_eq!(session.access_token, "token");
    assert!(
        request
            .await
            .unwrap()
            .starts_with("GET /app/consumeApp-consent?tokenId=value%26extra ")
    );
}

#[tokio::test]
async fn rejects_failed_consent_and_oversized_response() {
    let config = Config::new("123", "key", "secret").unwrap();
    for body in [
        r#"{"consentAppId":"id","status":"error","consentAppStatus":"GENERATED"}"#.to_owned(),
        r#"{"consentAppId":" ","status":"success","consentAppStatus":"GENERATED"}"#.to_owned(),
        "x".repeat(MAX_RESPONSE_BYTES + 1),
    ] {
        let (base, request) = mock_response("200 OK", &body).await;
        assert!(generate_consent(&config, &base).await.is_err());
        request.await.unwrap();
    }
}

#[tokio::test]
async fn rejects_invalid_callback_before_sending_request() {
    let config = Config::new("123", "key", "secret").unwrap();
    for token_id in ["".to_owned(), "bad\nvalue".to_owned(), "x".repeat(4097)] {
        assert!(
            exchange_token(&config, &token_id, "http://invalid.invalid")
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn rejects_http_errors_and_malformed_bodies_without_secret_leakage() {
    let config = Config::new("123", "key", "secret").unwrap();
    for (status, body) in [
        ("401 Unauthorized", "sensitive-value"),
        ("200 OK", "sensitive-value"),
        ("302 Found", "sensitive-value"),
    ] {
        let (base, request) = mock_response(status, body).await;
        let error = exchange_token(&config, "private-token-id", &base)
            .await
            .err()
            .unwrap();
        assert!(!format!("{error:#}").contains("sensitive-value"));
        assert!(!format!("{error:#}").contains("private-token-id"));
        request.await.unwrap();
    }
}

#[test]
fn parses_ist_expiry_and_rejects_invalid_calendar_dates() {
    assert_eq!(
        parse_expiry("2030-01-01T05:30:00.000").unwrap(),
        1_893_456_000
    );
    assert_eq!(parse_expiry("2030-01-01T00:00:00Z").unwrap(), 1_893_456_000);
    assert!(parse_expiry("2030-02-30T12:00:00").is_err());
    assert!(parse_expiry("").is_err());
}

#[test]
fn rejects_account_mismatch_empty_token_and_expired_response() {
    let config = Config::new("123", "key", "secret").unwrap();
    let response = |client: &str, token: &str, expiry: &str| TokenResponse {
        client_id: client.into(),
        access_token: token.into(),
        expiry_time: expiry.into(),
    };
    assert!(validate_session(&config, response("456", "token", "2030-01-01T00:00:00"), 0).is_err());
    assert!(validate_session(&config, response("123", " ", "2030-01-01T00:00:00"), 0).is_err());
    assert!(
        validate_session(
            &config,
            response("123", "token", "2000-01-01T00:00:00"),
            1_893_456_000
        )
        .is_err()
    );
    let session =
        validate_session(&config, response("123", "token", "2030-01-01T00:00:00"), 0).unwrap();
    assert_eq!(session.client_id, "123");
    assert_eq!(session.access_token, "token");
}
