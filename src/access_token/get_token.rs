use anyhow::{Context, Result, bail};
use reqwest::{Client, Url};
use serde::Deserialize;
use std::time::Duration;

use super::expiry::{
    Expiry, StatedExpiry, humanize, now_unix_seconds, parse_stated_expiry, resolve,
};
use super::session_file::{SessionRecord, cached_token, save_token};

const EXPIRY_MARGIN_SECONDS: i64 = 60;
const HTTP_TIMEOUT_SECONDS: u64 = 20;
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Deserialize)]
struct TokenResponse {
    #[serde(alias = "accessToken")]
    access_token: String,
    #[serde(default, alias = "expiryTime")]
    expires_at: StatedExpiry,
    #[serde(default, alias = "dhanClientId")]
    client_id: Option<String>,
}

struct FetchedToken {
    token: String,
    expiry: Expiry,
    fetched_at: i64,
    response: serde_json::Value,
}

pub(crate) async fn saved_token() -> Option<String> {
    let cached = cached_token().await.ok()??;
    let remaining = cached.expiry.remaining_seconds(now_unix_seconds().ok()?);
    (remaining > EXPIRY_MARGIN_SECONDS).then(|| {
        println!(
            "Reusing the saved Dhan access token, valid for another {}.",
            humanize(remaining)
        );
        cached.token
    })
}

pub(crate) async fn get_token(url: &str) -> Result<String> {
    let fetch_error = match fetch_token(url).await {
        Ok(fetched) => {
            report_validity(&fetched.expiry)?;
            let record = SessionRecord {
                access_token: &fetched.token,
                expiry: fetched.expiry,
                fetched_at: fetched.fetched_at,
                response: Some(fetched.response),
            };
            match save_token(record).await {
                Ok(path) => println!("Dhan access token saved to {}", path.display()),
                Err(error) => {
                    eprintln!("Warning: could not cache the Dhan access token: {error:#}");
                }
            }
            return Ok(fetched.token);
        }
        Err(error) => error,
    };

    let now = now_unix_seconds()?;
    match cached_token().await {
        Ok(Some(cached)) => {
            let remaining = cached.expiry.remaining_seconds(now);
            if remaining <= EXPIRY_MARGIN_SECONDS {
                let how = if cached.expiry.is_expired(now) {
                    format!("expired {} ago", humanize(remaining))
                } else {
                    format!("expires in {}", humanize(remaining))
                };
                return Err(fetch_error).context(format!(
                    "the cached Dhan access token is unusable: it {how} ({}). \
                     Dhan tokens are valid for 24 hours — reconnect Dhan, or set \
                     DHAN_ACCESS_TOKEN to a token from the Dhan dashboard",
                    cached.expiry.source.describe()
                ));
            }

            eprintln!("Warning: could not fetch a Dhan access token: {fetch_error:#}");
            eprintln!(
                "Warning: using the cached token, valid for another {} ({}).",
                humanize(remaining),
                cached.expiry.source.describe()
            );
            Ok(cached.token)
        }
        Ok(None) => Err(fetch_error).context(
            "no cached Dhan access token to fall back on (data/sessions/dhan_access_token.json does not exist)",
        ),
        Err(cache_error) => {
            eprintln!("Warning: the cached Dhan access token is unusable: {cache_error:#}");
            Err(fetch_error).context("could not fetch a token and the cached one is unusable")
        }
    }
}

fn report_validity(expiry: &Expiry) -> Result<()> {
    let now = now_unix_seconds()?;
    let remaining = expiry.remaining_seconds(now);

    if remaining <= EXPIRY_MARGIN_SECONDS {
        bail!(
            "the token route returned a token that is already expired or about to be ({} left, {})",
            humanize(remaining),
            expiry.source.describe()
        );
    }
    println!(
        "Dhan access token valid for another {} ({}).",
        humanize(remaining),
        expiry.source.describe()
    );
    Ok(())
}

async fn fetch_token(url: &str) -> Result<FetchedToken> {
    let passcode =
        std::env::var("TOKEN_PASSCODE").context("Missing TOKEN_PASSCODE environment variable")?;
    let client_id = std::env::var("DHAN_CLIENT_ID").ok();
    fetch_token_with_config(url, &passcode, client_id.as_deref()).await
}

async fn fetch_token_with_config(
    url: &str,
    passcode: &str,
    client_id: Option<&str>,
) -> Result<FetchedToken> {
    let url = validate_url(url)?;
    if passcode.trim().is_empty() || !passcode.bytes().all(|byte| byte.is_ascii_graphic()) {
        bail!("TOKEN_PASSCODE must be nonempty printable ASCII without whitespace");
    }
    let mut response = Client::builder()
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECONDS))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| anyhow::anyhow!("Could not initialize token fetcher client"))?
        .get(url)
        .header("x-token-passcode", passcode)
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("Failed to request access token"))?;

    let status = response.status();
    if !status.is_success() {
        bail!("Token fetcher returned HTTP {}", status.as_u16());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("Failed to read token fetcher response"))?
    {
        if chunk.len() > MAX_RESPONSE_BYTES.saturating_sub(body.len()) {
            bail!("Token fetcher response exceeded the size limit");
        }
        body.extend_from_slice(&chunk);
    }
    parse_response(&body, client_id)
}

fn parse_response(body: &[u8], client_id: Option<&str>) -> Result<FetchedToken> {
    let response: serde_json::Value = serde_json::from_slice(body)
        .map_err(|_| anyhow::anyhow!("Failed to parse token fetcher response as JSON"))?;
    let parsed = serde_json::from_value::<TokenResponse>(response.clone()).map_err(|_| {
        anyhow::anyhow!("Failed to read the access token out of the token fetcher response")
    })?;
    if let (Some(expected), Some(actual)) = (client_id, parsed.client_id.as_deref())
        && expected != actual
    {
        bail!("Token fetcher returned a different Dhan client ID");
    }
    let token = parsed.access_token.trim().to_owned();
    if token.is_empty() || !token.bytes().all(|byte| byte.is_ascii_graphic()) {
        bail!("Token fetcher returned an invalid access token");
    }

    let fetched_at = now_unix_seconds()?;
    let expiry = resolve(parse_stated_expiry(&parsed.expires_at), &token, fetched_at);
    Ok(FetchedToken {
        token,
        expiry,
        fetched_at,
        response,
    })
}

fn validate_url(input: &str) -> Result<Url> {
    let url = Url::parse(input).map_err(|_| anyhow::anyhow!("Invalid DHAN_TOKEN_URL"))?;
    let is_loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || (url.scheme() == "http" && is_loopback))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query_pairs().any(|(key, _)| {
            matches!(
                key.to_ascii_lowercase().as_str(),
                "passcode" | "token_passcode" | "x-token-passcode"
            )
        })
    {
        bail!(
            "DHAN_TOKEN_URL must use HTTPS (or loopback HTTP), without credentials, fragment, or passcode query parameters; use TOKEN_PASSCODE for the header"
        );
    }
    Ok(url)
}

#[cfg(test)]
#[path = "../../tests/access_token/get_token.rs"]
mod tests;
