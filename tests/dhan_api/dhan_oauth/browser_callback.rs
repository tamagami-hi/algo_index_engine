use super::*;

async fn available_redirect() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    format!(
        "http://127.0.0.1:{}/callback",
        listener.local_addr().unwrap().port()
    )
}

#[tokio::test]
async fn rejects_nonlocal_or_implicit_port_callbacks() {
    for url in [
        "https://example.com/callback",
        "http://0.0.0.0:8181/callback",
        "http://localhost:8181/callback",
        "http://127.0.0.1/callback",
        "not a URL",
        "file:///callback",
        "http://127.0.0.1:8181/?tokenId=x",
        "http://user:pass@127.0.0.1:8181/",
        "http://127.0.0.1:8181/#fragment",
    ] {
        assert!(bind(url).await.is_err());
    }
}

#[tokio::test]
async fn invalid_callbacks_do_not_end_login_and_valid_callback_hides_token() {
    let redirect = available_redirect().await;
    let listener = bind(&redirect).await.unwrap();
    let receiving = tokio::spawn(listener.receive());
    let client = reqwest::Client::new();
    let head = client
        .head(format!("{redirect}?tokenId=private%2Btoken"))
        .send()
        .await
        .unwrap();
    assert_eq!(head.status(), 405);
    let wrong_path = client
        .get(format!("{redirect}/wrong?tokenId=private%2Btoken"))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong_path.status(), 404);
    let invalid = client
        .get(format!("{redirect}?tokenId=a&tokenId=b"))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), 400);
    assert!(!receiving.is_finished());
    let valid = client
        .get(format!("{redirect}/?tokenId=private%2Btoken"))
        .send()
        .await
        .unwrap();
    assert_eq!(valid.status(), 200);
    assert_eq!(valid.headers()["cache-control"], "no-store");
    assert!(!valid.text().await.unwrap().contains("private+token"));
    let callback = receiving.await.unwrap().unwrap();
    assert_eq!(callback, "private+token");
}

#[test]
fn decodes_exactly_one_nonempty_printable_token() {
    let redirect = Url::parse("http://127.0.0.1:8080/callback").unwrap();
    assert_eq!(callback_token("tokenId=a%2Bb", &redirect).unwrap(), "a+b");
    for query in [
        "",
        "error=denied",
        "tokenId=",
        "tokenId=a&tokenId=b",
        "tokenId=secret&token%49d=other",
        "tokenId=secret%20value",
        "tokenId=secret%00",
        "tokenId=secret%0A",
        "tokenId=%C3%A9",
        "tokenId=secret&unexpected=value",
    ] {
        let error = callback_token(query, &redirect).unwrap_err();
        assert!(!format!("{error:#}").contains("secret"));
    }
}

#[test]
fn preserves_registered_query_pairs_and_their_multiplicity() {
    let redirect =
        Url::parse("http://127.0.0.1:8080/callback?app=dhan&app=dhan&mode=login").unwrap();
    assert!(callback_token("mode=login&tokenId=x&app=dhan&app=dhan", &redirect).is_ok());
    for query in [
        "tokenId=x",
        "app=other&app=dhan&mode=login&tokenId=x",
        "app=dhan&mode=login&tokenId=x",
        "app=dhan&app=dhan&app=dhan&mode=login&tokenId=x",
        "app=dhan&mode=login&mode=login&tokenId=x",
    ] {
        assert!(callback_token(query, &redirect).is_err());
    }
}

#[test]
fn limits_the_complete_callback_url_to_8192_bytes() {
    const CALLBACK_LIMIT: usize = 8192;
    let redirect = Url::parse("http://127.0.0.1:8080/callback").unwrap();
    let token_length = CALLBACK_LIMIT - redirect.as_str().len() - "?tokenId=".len();
    let at_limit = format!("tokenId={}", "a".repeat(token_length));
    assert!(callback_token(&at_limit, &redirect).is_ok());
    assert!(callback_token(&format!("{at_limit}a"), &redirect).is_err());
}
