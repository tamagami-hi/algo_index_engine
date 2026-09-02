use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;

const TOKEN_PASSCODE_HEADER: &str = "x-token-passcode";

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

pub(crate) async fn get_token() -> Result<String> {
    let url =
        std::env::var("KITE_TOKEN_URL").context("Missing KITE_TOKEN_URL environment variable")?;
    let passcode = std::env::var("KITE_TOKEN_PASSCODE")
        .context("Missing KITE_TOKEN_PASSCODE environment variable")?;

    let token = Client::new()
        .get(url)
        .header(TOKEN_PASSCODE_HEADER, passcode)
        .send()
        .await
        .context("Failed to request Kite access token")?
        .error_for_status()
        .context("Token fetcher returned an error")?
        .json::<TokenResponse>()
        .await
        .context("Failed to parse token fetcher response")?;

    Ok(token.access_token)
}
