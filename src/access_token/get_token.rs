use anyhow::{Context, Result, bail};
use reqwest::{Client, Url};
use serde::Deserialize;
use std::time::Duration;

use super::expiry::{
    Expiry, StatedExpiry, humanize, now_unix_seconds, parse_stated_expiry, resolve,
};
use super::session_file::{SessionRecord, cached_token, save_token};

/// A token this close to expiry is treated as already gone.
///
/// Startup reads the instrument master and opens five feed connections, so a token
/// with seconds left would be accepted here and then rejected mid-handshake. Better to
/// fail while the reason is still obvious.
const EXPIRY_MARGIN_SECONDS: i64 = 60;
const HTTP_TIMEOUT_SECONDS: u64 = 20;
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// What the token route returns.
///
/// The Cal Spread route sends `access_token` plus `expires_at` (epoch ms), `client_id`
/// and `login_date`. Dhan's own endpoints — `generateAccessToken`, `consumeApp-consent`
/// and `partner/consume-consent` — send `accessToken` and `expiryTime` (an ISO stamp in
/// IST) alongside `dhanClientId`, `dhanClientName`, `dhanClientUcc` and
/// `givenPowerOfAttorney`.
///
/// Both spellings are accepted so `DHAN_TOKEN_URL` can point at either without a code
/// change. When present, the returned client ID must match `DHAN_CLIENT_ID`.
#[derive(Deserialize)]
struct TokenResponse {
    #[serde(alias = "accessToken")]
    access_token: String,
    /// `expires_at` from the Cal Spread route, `expiryTime` from Dhan.
    #[serde(default, alias = "expiryTime")]
    expires_at: StatedExpiry,
    #[serde(default, alias = "dhanClientId")]
    client_id: Option<String>,
}

/// A token plus when it stops being usable, and the reply it came from.
struct FetchedToken {
    token: String,
    expiry: Expiry,
    fetched_at: i64,
    /// The route's reply, verbatim, for the cache to record.
    response: serde_json::Value,
}

/// Obtain a Dhan access token, falling back to the cached one while it is still valid.
///
/// The token route depends on an upstream Dhan session that is not always live — it
/// answers 409 when the admin has not connected Dhan — and that says nothing about
/// whether the token we already hold is still good. So a fetch failure is not fatal
/// while an unexpired cached token exists.
///
/// A cached token past its expiry is REFUSED, not used with a warning. Dhan issues
/// tokens for 24 hours; sending an expired one produces opaque rejections at the feed
/// handshake, far from the actual cause. Failing here names the cause once.
///
/// A freshly fetched token is written to the cache; a token that CAME from the cache is
/// not written back, since rewriting it would serve no purpose and only churn the file.
pub(crate) async fn get_token(url: &str) -> Result<String> {
    let fetch_error = match fetch_token(url).await {
        Ok(fetched) => {
            report_validity(&fetched.expiry)?;
            // A cache write failure must not sink a good token: report and continue.
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
        // The cache is unusable AND the fetch failed. Surface the fetch failure as the
        // cause, since that is the problem to fix, and mention the cache separately.
        Err(cache_error) => {
            eprintln!("Warning: the cached Dhan access token is unusable: {cache_error:#}");
            Err(fetch_error).context("could not fetch a token and the cached one is unusable")
        }
    }
}

/// Reject a freshly fetched token that is already expired, and say how long a good one
/// has left.
///
/// The route should never hand out an expired token, but trusting that silently would
/// turn a server-side bug into an unexplained feed rejection.
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

/// Request a fresh token from the token route.
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
    // Kept as a Value first so the reply can be recorded exactly as it arrived, then
    // read into the typed shape. Deserialising from the Value rather than re-parsing
    // the text guarantees the two cannot disagree.
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
