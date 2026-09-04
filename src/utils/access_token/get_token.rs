use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::Deserialize;

use super::session_file::{cached_token, save_token};

/// One IST trading day. Dhan access tokens are day-scoped, so a cached token older
/// than this is very unlikely to still be accepted at the feed handshake.
const STALE_AFTER_SECONDS: u64 = 24 * 60 * 60;

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

/// Error shape returned by the Cal Spread token route, e.g. a 409 with
/// `{"authenticated":false,"error":"No live Dhan session..."}`.
#[derive(Deserialize)]
struct TokenErrorResponse {
    error: String,
}

/// Obtain a Dhan access token, falling back to the cached one.
///
/// The token route depends on an upstream Dhan session that is not always live — it
/// answers 409 when the admin has not connected Dhan — and that has nothing to do with
/// whether the token we already hold is still good. So a fetch failure is not fatal
/// while a cached token exists.
///
/// A freshly fetched token is written to the cache; a token that CAME from the cache is
/// not written back. Re-writing it would refresh the file's timestamp and make a token
/// from days ago look brand new, destroying the only staleness signal there is.
pub(crate) async fn get_token(url: &str) -> Result<String> {
    let fetch_error = match fetch_token(url).await {
        Ok(token) => {
            // A cache write failure must not sink a good token: report and continue.
            match save_token(&token).await {
                Ok(path) => println!("Dhan access token saved to {}", path.display()),
                Err(error) => {
                    eprintln!("Warning: could not cache the Dhan access token: {error:#}");
                }
            }
            return Ok(token);
        }
        Err(error) => error,
    };

    match cached_token().await {
        Ok(Some(cached)) => {
            eprintln!("Warning: could not fetch a Dhan access token: {fetch_error:#}");
            eprintln!("Warning: falling back to the cached token{}.", age_note(&cached));
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

/// Describe a cached token's age, flagging one that is almost certainly expired.
fn age_note(cached: &super::session_file::CachedToken) -> String {
    match cached.age {
        Some(age) if age.as_secs() >= STALE_AFTER_SECONDS => format!(
            ", which is {} old and probably expired — the feed will likely reject it",
            humanize(age.as_secs())
        ),
        Some(age) => format!(", cached {} ago", humanize(age.as_secs())),
        None => String::new(),
    }
}

fn humanize(seconds: u64) -> String {
    match seconds {
        0..=119 => format!("{seconds}s"),
        120..=7199 => format!("{}m", seconds / 60),
        7200..=172_799 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86_400),
    }
}

/// Request a fresh token from the token route.
async fn fetch_token(url: &str) -> Result<String> {
    let passcode =
        std::env::var("TOKEN_PASSCODE").context("Missing TOKEN_PASSCODE environment variable")?;

    let response = Client::new()
        .get(url)
        .header("x-token-passcode", passcode)
        .send()
        .await
        .context("Failed to request access token")?;

    // Read the body before failing, so the token service explains *why* it refused
    // instead of surfacing a bare HTTP status.
    let status = response.status();
    let body = response
        .text()
        .await
        .context("Failed to read token fetcher response")?;

    if !status.is_success() {
        let reason = serde_json::from_str::<TokenErrorResponse>(&body)
            .map(|parsed| parsed.error)
            .unwrap_or_else(|_| body.trim().to_owned());
        bail!("Token fetcher returned HTTP {status}: {reason}");
    }

    serde_json::from_str::<TokenResponse>(&body)
        .map(|token| token.access_token)
        .context("Failed to parse token fetcher response")
}
