use super::*;

fn config() -> Config {
    Config::new("100001", "app", "secret").unwrap()
}
fn session() -> DhanSession {
    DhanSession {
        client_id: "100001".into(),
        api_key: "app".into(),
        access_token: "token".into(),
        expires_at: 2000,
    }
}

#[test]
fn round_trip_reuses_only_unexpired_matching_account_and_app() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sessions/oauth.json");
    assert!(load(&path, &config(), 1000).unwrap().is_none());
    save(&path, &session()).unwrap();
    assert_eq!(
        load(&path, &config(), 1000).unwrap().unwrap().access_token,
        "token"
    );
    assert!(load(&path, &config(), 1940).unwrap().is_none());
    let other = Config::new("100002", "app", "secret").unwrap();
    assert!(load(&path, &other, 1000).unwrap().is_none());
    let other_app = Config::new("100001", "other", "secret").unwrap();
    assert!(load(&path, &other_app, 1000).unwrap().is_none());
}

#[test]
fn rejects_corrupt_or_empty_session_without_disclosing_contents() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("oauth.json");
    std::fs::write(&path, "super-secret-not-json").unwrap();
    assert!(!format!("{:#}", load(&path, &config(), 1000).err().unwrap()).contains("super-secret"));
    let empty = DhanSession {
        access_token: " ".into(),
        ..session()
    };
    save(&path, &empty).unwrap();
    assert!(load(&path, &config(), 1000).unwrap().is_none());
}

#[test]
fn replacing_session_preserves_valid_json() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("oauth.json");
    save(&path, &session()).unwrap();
    let next = DhanSession {
        access_token: "next".into(),
        ..session()
    };
    save(&path, &next).unwrap();
    assert_eq!(
        load(&path, &config(), 1000).unwrap().unwrap().access_token,
        "next"
    );
}

#[cfg(unix)]
#[test]
fn restricts_permissions_and_refuses_symlink_targets() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sessions/oauth.json");
    save(&path, &session()).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let link = dir.path().join("link.json");
    symlink(&path, &link).unwrap();
    assert!(save(&link, &session()).is_err());
    assert!(load(&link, &config(), 1000).is_err());
    let link_dir = dir.path().join("linked");
    symlink(path.parent().unwrap(), &link_dir).unwrap();
    assert!(save(&link_dir.join("oauth.json"), &session()).is_err());
}
