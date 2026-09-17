use super::*;

#[test]
fn environment_dispatch_is_isolated_and_honors_explicit_modes() {
    for mode in ["manual", "unset", "invalid", "token_url", "web"] {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "dhan_api::dhan_auth::tests::isolated_environment_child",
            ])
            .env("DHAN_TEST_CHILD", mode)
            .env_remove("DHAN_AUTH_MODE")
            .envs((mode != "unset").then_some(("DHAN_AUTH_MODE", mode)))
            .env("DHAN_ACCESS_TOKEN", "test-token")
            .env("DHAN_CLIENT_ID", "123")
            .env("DHAN_API_KEY", "test-key")
            .env_remove("DHAN_API_SECRET")
            .env_remove("DHAN_TOKEN_URL")
            .env_remove("DHAN_REDIRECT_URL")
            .status()
            .unwrap();
        assert!(status.success());
    }
}

#[tokio::test]
async fn isolated_environment_child() {
    let Ok(mode) = std::env::var("DHAN_TEST_CHILD") else {
        return;
    };
    let result = get_dhan_credentials(None).await;
    if matches!(mode.as_str(), "manual" | "unset") {
        let credentials = result.unwrap();
        assert_eq!(credentials.access_token, "test-token");
        assert_eq!(credentials.client_id, "123");
        assert_eq!(credentials.api_key, "test-key");
    } else if mode == "web" {
        assert!(
            result
                .err()
                .unwrap()
                .to_string()
                .contains("DHAN_API_SECRET")
        );
    } else {
        assert!(result.is_err());
    }
}

#[test]
fn explicit_modes_override_any_manual_token() {
    for (mode, expected) in [
        ("web", AuthMode::Web),
        ("oauth", AuthMode::Web),
        ("token_url", AuthMode::TokenUrl),
        ("manual", AuthMode::Manual),
    ] {
        assert_eq!(select_mode(Some(mode), Some("token")).unwrap(), expected);
        assert_eq!(select_mode(Some(mode), None).unwrap(), expected);
    }
}

#[test]
fn unset_or_blank_mode_preserves_legacy_selection() {
    for mode in [None, Some(""), Some("  ")] {
        assert_eq!(
            select_mode(mode, Some(" token ")).unwrap(),
            AuthMode::Manual
        );
        assert_eq!(select_mode(mode, Some(" \n")).unwrap(), AuthMode::TokenUrl);
        assert_eq!(select_mode(mode, None).unwrap(), AuthMode::TokenUrl);
    }
}

#[test]
fn invalid_mode_has_actionable_error_without_echoing_input() {
    let error = select_mode(Some("sensitive-invalid-value"), None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("DHAN_AUTH_MODE"));
    assert!(error.contains("web, token_url, or manual"));
    assert!(!error.contains("sensitive-invalid-value"));
}

#[test]
fn manual_mode_requires_a_valid_token() {
    assert_eq!(manual_token(Some(" token ")).unwrap(), "token");
    for token in [
        None,
        Some(""),
        Some(" \n"),
        Some("bad\nvalue"),
        Some("bad value"),
    ] {
        assert!(manual_token(token).is_err());
    }
}
