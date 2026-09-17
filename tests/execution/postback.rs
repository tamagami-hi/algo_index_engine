use super::*;

use crate::config::sandbox::Sandbox;

fn today() -> String {
    crate::dhan_api::instruments::ist_today().expect("ist today")
}

fn journal_lines() -> Vec<serde_json::Value> {
    let path = journal_path(&today());
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("journal line is not JSON: {error}: {line}"))
        })
        .collect()
}

#[test]
fn the_body_is_stored_byte_for_byte() {
    let _sandbox = Sandbox::new();
    let body = r#"{"dhanClientId":"1000000009","orderId":"112111182198","orderStatus":"TRADED","filled_qty":"50"}"#;

    let stored = record(body, Some("100.64.0.1")).expect("record");
    assert_eq!(stored.records, 1);
    assert_eq!(stored.day, today());

    let lines = journal_lines();
    assert_eq!(lines.len(), 1, "one postback writes one line");
    assert_eq!(
        lines[0]["body"].as_str(),
        Some(body),
        "the broker's own bytes are kept verbatim, not reserialised"
    );
    assert_eq!(lines[0]["bytes"].as_u64(), Some(body.len() as u64));
    assert_eq!(lines[0]["source"].as_str(), Some("100.64.0.1"));
    assert!(
        lines[0]["received_at_ms"].as_i64().unwrap_or(0) > 0,
        "every record carries the moment it arrived"
    );
}

#[test]
fn a_body_that_is_not_json_is_still_kept() {
    let _sandbox = Sandbox::new();
    let body = "orderStatus=TRADED&orderId=112111182198";

    record(body, None).expect("record");

    let lines = journal_lines();
    assert_eq!(
        lines[0]["body"].as_str(),
        Some(body),
        "nothing here parses the payload, so nothing here can reject one"
    );
    assert!(
        lines[0]["source"].is_null(),
        "an unknown sender is recorded as unknown, not invented"
    );
}

#[test]
fn a_body_carrying_json_control_characters_survives_the_round_trip() {
    let _sandbox = Sandbox::new();
    let body = "{\"omsErrorDescription\":\"line one\nline two\ttabbed \\\"quoted\\\"\"}";

    record(body, None).expect("record");

    let lines = journal_lines();
    assert_eq!(
        lines[0]["body"].as_str(),
        Some(body),
        "a newline inside the payload must not become a second journal line"
    );
    assert_eq!(
        lines.len(),
        1,
        "the journal stays one line per postback whatever the payload contains"
    );
}

#[test]
fn every_postback_appends_rather_than_replacing() {
    let _sandbox = Sandbox::new();

    for status in ["TRANSIT", "PENDING", "TRADED"] {
        let stored = record(&format!("{{\"orderStatus\":\"{status}\"}}"), None).expect("record");
        assert_eq!(
            stored.records,
            match status {
                "TRANSIT" => 1,
                "PENDING" => 2,
                _ => 3,
            },
            "the count reported back is the count on disk"
        );
    }

    let lines = journal_lines();
    assert_eq!(lines.len(), 3);
    let statuses: Vec<String> = lines
        .iter()
        .map(|line| {
            let body: serde_json::Value =
                serde_json::from_str(line["body"].as_str().expect("body")).expect("body json");
            body["orderStatus"].as_str().expect("status").to_owned()
        })
        .collect();
    assert_eq!(
        statuses,
        vec!["TRANSIT", "PENDING", "TRADED"],
        "the order of arrival is the order on disk"
    );
}

#[test]
fn concurrent_arrivals_all_land_and_none_interleave() {
    let _sandbox = Sandbox::new();
    let writers = 8;
    let each = 16;

    std::thread::scope(|scope| {
        for writer in 0..writers {
            scope.spawn(move || {
                for sequence in 0..each {
                    record(
                        &format!("{{\"writer\":{writer},\"sequence\":{sequence}}}"),
                        None,
                    )
                    .expect("record");
                }
            });
        }
    });

    let lines = journal_lines();
    assert_eq!(
        lines.len(),
        writers * each,
        "no concurrent append may be lost or overwritten"
    );

    let mut seen = std::collections::BTreeSet::new();
    for line in &lines {
        let body: serde_json::Value =
            serde_json::from_str(line["body"].as_str().expect("body")).expect("body json");
        let pair = (
            body["writer"].as_u64().expect("writer"),
            body["sequence"].as_u64().expect("sequence"),
        );
        assert!(seen.insert(pair), "{pair:?} was written twice");
    }
    assert_eq!(seen.len(), writers * each);
}

#[test]
fn the_journal_directory_is_created_on_first_arrival() {
    let _sandbox = Sandbox::new();
    let path = journal_path(&today());
    let parent = path.parent().expect("parent");
    assert!(!parent.exists(), "the sandbox starts without a journal");

    record("{}", None).expect("record");

    assert!(parent.is_dir(), "the first postback creates its directory");
    assert!(path.is_file());
}

#[test]
fn the_journal_is_named_for_the_trading_day() {
    let _sandbox = Sandbox::new();
    let path = journal_path("2026-09-16");
    assert!(
        path.ends_with("data/execution/postbacks-2026-09-16.jsonl"),
        "one file per day keeps a day's orders together: {}",
        path.display()
    );
}

#[test]
fn a_reader_can_take_what_is_there_while_more_is_arriving() {
    let _sandbox = Sandbox::new();
    record("{\"orderStatus\":\"PENDING\"}", None).expect("record");

    let path = journal_path(&today());
    let partial = std::fs::read_to_string(&path).expect("read");
    assert!(
        partial.ends_with('\n'),
        "a complete record always ends its line, so a reader can tell it is complete"
    );

    record("{\"orderStatus\":\"TRADED\"}", None).expect("record");
    let both = std::fs::read_to_string(&path).expect("read");
    assert!(
        both.starts_with(&partial),
        "an append never rewrites what a reader has already taken"
    );
}

static URL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn with_url<T>(value: Option<&str>, body: impl FnOnce() -> T) -> T {
    let _guard = URL_LOCK.lock().unwrap_or_else(|error| {
        URL_LOCK.clear_poison();
        error.into_inner()
    });
    unsafe {
        match value {
            Some(value) => std::env::set_var(POSTBACK_URL_VARIABLE, value),
            None => std::env::remove_var(POSTBACK_URL_VARIABLE),
        }
    }
    let outcome = body();
    unsafe {
        std::env::remove_var(POSTBACK_URL_VARIABLE);
    }
    outcome
}

fn listener() -> std::net::SocketAddr {
    "127.0.0.1:8787".parse().expect("addr")
}

#[test]
fn an_unset_url_is_not_an_error() {
    for value in [None, Some(""), Some("   ")] {
        let outcome = with_url(value, || configured(listener()));
        assert_eq!(
            outcome.expect("unset is allowed"),
            PostbackUrl::Unset,
            "the engine has to start before an operator has registered a URL: {value:?}"
        );
    }
}

#[test]
fn a_url_pointing_at_the_wrong_path_is_refused_at_startup() {
    for value in [
        "http://127.0.0.1:8787/dhan/callback",
        "http://127.0.0.1:8787/postback",
        "http://127.0.0.1:8787/dhan/postback/",
        "http://127.0.0.1:8787/",
    ] {
        let error = with_url(Some(value), || configured(listener()))
            .expect_err("a path the engine does not serve must be refused");
        assert!(
            format!("{error:#}").contains(POSTBACK_PATH),
            "the refusal has to name the path the engine actually serves: {error:#}"
        );
    }
}

#[test]
fn a_loopback_url_must_name_the_listener_port() {
    let matching = with_url(Some("http://127.0.0.1:8787/dhan/postback"), || {
        configured(listener())
    })
    .expect("a matching loopback port is accepted");
    assert_eq!(
        matching,
        PostbackUrl::LocalOnly,
        "Dhan refuses loopback URLs, so a matching one is still only good for local testing"
    );

    let mismatched = with_url(Some("http://127.0.0.1:9999/dhan/postback"), || {
        configured(listener())
    });
    assert!(
        mismatched.is_err(),
        "a postback URL on a port the engine does not serve would answer nothing"
    );

    let portless = with_url(Some("http://localhost/dhan/postback"), || {
        configured(listener())
    });
    assert!(
        portless.is_err(),
        "a loopback URL without a port cannot be checked against the listener"
    );
}

#[test]
fn a_public_url_is_accepted_without_a_port_check() {
    for value in [
        "https://index.algo.example/dhan/postback",
        "http://203.0.113.7/dhan/postback",
        "https://index.algo.example:8443/dhan/postback",
    ] {
        let outcome = with_url(Some(value), || configured(listener()))
            .unwrap_or_else(|error| panic!("{value} should be accepted: {error:#}"));
        assert_eq!(
            outcome,
            PostbackUrl::Reachable,
            "behind an edge the public port is not the loopback port: {value}"
        );
    }
}

#[test]
fn a_url_that_is_not_usable_http_is_refused() {
    for value in [
        "not a url",
        "ftp://127.0.0.1:8787/dhan/postback",
        "file:///dhan/postback",
        "http://127.0.0.1:8787/dhan/postback?token=secret",
        "http://127.0.0.1:8787/dhan/postback#fragment",
    ] {
        assert!(
            with_url(Some(value), || configured(listener())).is_err(),
            "{value} must not be accepted as a postback URL"
        );
    }
}
