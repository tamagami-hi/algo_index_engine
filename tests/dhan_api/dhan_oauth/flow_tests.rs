use reqwest::Url;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

use super::{browser_callback::callback_token, server_callbacks, session, types::Config};
use crate::dhan_api::dhan_auth::DhanCredentials;

async fn mock_broker() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        for (request_line, body) in [
            (
                "POST /app/generate-consent?client_id=100001 HTTP/1.1",
                r#"{"consentAppId":"test-consent","consentAppStatus":"GENERATED","status":"success"}"#,
            ),
            (
                "GET /app/consumeApp-consent?tokenId=callback%2Btoken HTTP/1.1",
                r#"{"dhanClientId":"100001","accessToken":"test-access-token","expiryTime":"2099-01-01T05:30:00"}"#,
            ),
        ] {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 4096];
            let size = socket.read(&mut buffer).await.unwrap();
            let request = std::str::from_utf8(&buffer[..size]).unwrap();
            assert!(request.starts_with(request_line));
            assert!(request.contains("app_id: test-app\r\n"));
            assert!(request.contains("app_secret: test-secret\r\n"));
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(reply.as_bytes()).await.unwrap();
        }
    });
    (base, task)
}

#[tokio::test]
async fn complete_browser_consent_persists_and_reuses_matching_session() {
    let config = Config::new("100001", "test-app", "test-secret").unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sessions/oauth.json");
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    assert!(session::load(&path, &config, now).unwrap().is_none());
    let (base, broker) = mock_broker().await;

    let login_url = server_callbacks::generate_consent(&config, &base)
        .await
        .unwrap();
    let parsed = Url::parse(&login_url).unwrap();
    assert_eq!(parsed.host_str(), Some("auth.dhan.co"));
    assert_eq!(parsed.path(), "/login/consentApp-login");
    assert_eq!(parsed.query(), Some("consentAppId=test-consent"));
    let token_id = callback_token(
        "tokenId=callback%2Btoken",
        &Url::parse("http://127.0.0.1:8080/callback").unwrap(),
    )
    .unwrap();
    let authenticated = server_callbacks::exchange_token(&config, &token_id, &base)
        .await
        .unwrap();
    session::save(&path, &authenticated).unwrap();
    let credentials = DhanCredentials::from(authenticated);
    assert_eq!(credentials.client_id, "100001");
    assert_eq!(credentials.api_key, "test-app");
    assert_eq!(credentials.access_token, "test-access-token");
    broker.await.unwrap();

    let reused = session::load(&path, &config, now).unwrap().unwrap();
    assert_eq!(reused.access_token, "test-access-token");
    assert_eq!(reused.client_id, "100001");
    assert_eq!(reused.expires_at, 4_070_908_800);
    assert!(
        session::load(&path, &config, reused.expires_at)
            .unwrap()
            .is_none()
    );
    let wrong_account = Config::new("100002", "test-app", "test-secret").unwrap();
    assert!(session::load(&path, &wrong_account, now).unwrap().is_none());
    let wrong_app = Config::new("100001", "another-app", "test-secret").unwrap();
    assert!(session::load(&path, &wrong_app, now).unwrap().is_none());
    let credentials = DhanCredentials::from(reused);
    assert_eq!(credentials.client_id, "100001");
    assert_eq!(credentials.api_key, "test-app");
    assert_eq!(credentials.access_token, "test-access-token");
}

#[tokio::test]
async fn shared_backend_callback_completes_broker_exchange_and_session_reuse() {
    use super::shared_callback::SharedCallback;
    use axum::routing::get;

    let config = Config::new("100001", "test-app", "test-secret").unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sessions/oauth.json");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let backend = format!("http://{}", listener.local_addr().unwrap());
    let callback = SharedCallback::new(&format!("{backend}/dhan/callback")).unwrap();
    let router = callback.router().route("/health", get(|| async { "ok" }));
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let pending = callback.begin().unwrap();
    let (base, broker) = mock_broker().await;
    let login_url = server_callbacks::generate_consent(&config, &base)
        .await
        .unwrap();
    assert_eq!(
        Url::parse(&login_url).unwrap().query(),
        Some("consentAppId=test-consent")
    );

    let client = reqwest::Client::new();
    assert_eq!(
        client
            .get(format!("{backend}/health"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "ok"
    );
    assert!(
        client
            .get(format!("{backend}/dhan/callback?tokenId=callback%2Btoken"))
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    let token = pending.receive().await.unwrap();
    let authenticated = server_callbacks::exchange_token(&config, &token, &base)
        .await
        .unwrap();
    session::save(&path, &authenticated).unwrap();
    broker.await.unwrap();
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let reused = session::load(&path, &config, now).unwrap().unwrap();
    assert_eq!(reused.access_token, "test-access-token");
    assert!(
        client
            .get(format!("{backend}/health"))
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    server.abort();
}
