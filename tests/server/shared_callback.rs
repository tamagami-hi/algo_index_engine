use super::*;
use crate::dhan_api::dhan_oauth::shared_callback::SharedCallback;

#[test]
fn listener_address_requires_explicit_valid_environment_configuration() {
    for value in [
        None,
        Some(""),
        Some("127.0.0.1:0"),
        Some("invalid"),
        Some("127.0.0.1:49234"),
    ] {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "server::http::shared_callback_tests::listener_environment_child",
            ])
            .env("LISTENER_TEST_CHILD", "1")
            .env_remove("BLACKBOX_HTTP_ADDR")
            .envs(value.map(|value| ("BLACKBOX_HTTP_ADDR", value)))
            .status()
            .unwrap();
        assert!(status.success(), "listener configuration case failed");
    }
}

#[test]
fn listener_environment_child() {
    if std::env::var("LISTENER_TEST_CHILD").is_err() {
        return;
    }
    let result = listen_addr();
    if std::env::var("BLACKBOX_HTTP_ADDR").as_deref() == Ok("127.0.0.1:49234") {
        assert_eq!(result.unwrap().port(), 49234);
    } else {
        assert!(result.is_err());
    }
}

#[tokio::test]
async fn backend_health_and_dhan_callback_share_one_listener_during_login() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let callback = SharedCallback::new(&format!("{origin}/dhan/callback")).unwrap();
    let pending = callback.begin().unwrap();
    let shutdown = CancellationToken::new();
    let server = tokio::spawn(serve(
        EngineState::new(),
        listener,
        shutdown.clone(),
        Some(callback.clone()),
    ));
    let client = reqwest::Client::new();

    let health = client.get(format!("{origin}/health")).send().await.unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    let ready = client.get(format!("{origin}/ready")).send().await.unwrap();
    assert_eq!(ready.status(), StatusCode::SERVICE_UNAVAILABLE);
    let response = client
        .get(format!("{origin}/dhan/callback?tokenId=test-token"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(pending.receive().await.unwrap(), "test-token");
    let idle = client
        .get(format!("{origin}/dhan/callback?tokenId=another-token"))
        .send()
        .await
        .unwrap();
    assert!(!idle.status().is_success());
    let state = client
        .get(format!("{origin}/api/state"))
        .send()
        .await
        .unwrap();
    assert_eq!(state.status(), StatusCode::OK);

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
