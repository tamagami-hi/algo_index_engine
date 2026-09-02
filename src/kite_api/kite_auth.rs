use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use futures_util::TryStreamExt;
use reqwest::{
    Client, StatusCode, Url,
    header::{ACCEPT, ACCEPT_ENCODING, CONTENT_ENCODING, HeaderValue},
    redirect::Policy,
};
use serde::Deserialize;

const APPROVED_BROKER_HOST: &str = "calspread.online";
const APPROVED_BROKER_PATH: &str = "/api/kite/token";
const TOKEN_PASSCODE_HEADER: &str = "x-token-passcode";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RESPONSE_BYTES: usize = 8 * 1_024;
const MAX_ACCESS_TOKEN_CHARS: usize = 2_048;

#[derive(Deserialize)]
struct TokenResponse {
    authenticated: bool,
    access_token: Option<String>,
}

pub(crate) async fn get_token() -> Result<String> {
    let raw_url =
        std::env::var("KITE_TOKEN_URL").context("Missing KITE_TOKEN_URL environment variable")?;
    let passcode = std::env::var("KITE_TOKEN_PASSCODE")
        .context("Missing KITE_TOKEN_PASSCODE environment variable")?;

    ensure!(
        !passcode.trim().is_empty(),
        "KITE_TOKEN_PASSCODE must not be blank"
    );

    let url = validate_broker_url(&raw_url)?;
    let client = build_client()?;
    let passcode_header = sensitive_header_value(&passcode)?;

    let response = client
        .get(url)
        .header(TOKEN_PASSCODE_HEADER, passcode_header)
        .header(ACCEPT, "application/json")
        .header(ACCEPT_ENCODING, "identity")
        .send()
        .await
        .map_err(reqwest::Error::without_url)
        .context("Token fetcher is unavailable")?;

    let status = response.status();
    if status != StatusCode::OK && status != StatusCode::CONFLICT {
        bail!("Token fetcher request failed with HTTP status {status}");
    }

    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        bail!("Token fetcher response is too large");
    }

    if response
        .headers()
        .get(CONTENT_ENCODING)
        .is_some_and(|value| !value.as_bytes().eq_ignore_ascii_case(b"identity"))
    {
        bail!("Token fetcher returned an encoded response");
    }

    let response_body = response
        .bytes_stream()
        .map_err(anyhow::Error::from)
        .try_fold(Vec::new(), |body, chunk| async move {
            ensure!(
                chunk.len() <= MAX_RESPONSE_BYTES.saturating_sub(body.len()),
                "Token fetcher response is too large"
            );

            Ok::<_, anyhow::Error>(body.into_iter().chain(chunk).collect())
        })
        .await
        .context("Failed to read token fetcher response")?;

    let payload: TokenResponse = serde_json::from_slice(&response_body)
        .context("Token fetcher returned an invalid response")?;

    extract_access_token(status, payload)
}

fn build_client() -> Result<Client> {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(Policy::none())
        .no_proxy()
        .https_only(true)
        .build()
        .context("Failed to create token fetcher client")
}

fn sensitive_header_value(value: &str) -> Result<HeaderValue> {
    let mut header =
        HeaderValue::from_str(value).context("KITE_TOKEN_PASSCODE is not a valid header value")?;
    header.set_sensitive(true);
    Ok(header)
}

fn validate_broker_url(raw_url: &str) -> Result<Url> {
    let url = Url::parse(raw_url).context("KITE_TOKEN_URL is not a valid URL")?;

    ensure!(
        url.scheme() == "https"
            && url.host_str() == Some(APPROVED_BROKER_HOST)
            && url.port_or_known_default() == Some(443)
            && url.path() == APPROVED_BROKER_PATH
            && url.query().is_none()
            && url.fragment().is_none()
            && url.username().is_empty()
            && url.password().is_none(),
        "KITE_TOKEN_URL must be the approved HTTPS token endpoint"
    );

    Ok(url)
}

fn extract_access_token(status: StatusCode, payload: TokenResponse) -> Result<String> {
    if !payload.authenticated {
        ensure!(
            payload.access_token.is_none(),
            "Token fetcher returned an invalid response"
        );
        bail!("Token fetcher has no active Kite session");
    }

    ensure!(
        status == StatusCode::OK,
        "Token fetcher returned an invalid response"
    );

    let access_token = payload
        .access_token
        .context("Token fetcher response is missing the access token")?;

    ensure!(
        access_token.chars().count() <= MAX_ACCESS_TOKEN_CHARS,
        "Token fetcher returned an invalid access token"
    );

    let access_token = access_token.trim();

    ensure!(
        !access_token.is_empty(),
        "Token fetcher returned an invalid access token"
    );

    Ok(access_token.to_owned())
}