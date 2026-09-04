use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::Deserialize;

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}


#[derive(Deserialize)]
struct TokenErrorResponse {
    error: String,
}

pub(crate) async fn get_token(url: &str) -> Result<String> {
    let passcode =
        std::env::var("TOKEN_PASSCODE").context("Missing TOKEN_PASSCODE environment variable")?;

    let response = Client::new()
        .get(url)
        .header("x-token-passcode", passcode)
        .send()
        .await
        .context("Failed to request access token")?;

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
