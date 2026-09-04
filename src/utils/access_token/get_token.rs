use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

pub(crate) async fn get_token(url: &str) -> Result<String> {
    let passcode =
        std::env::var("TOKEN_PASSCODE").context("Missing TOKEN_PASSCODE environment variable")?;

    let token = Client::new()
        .get(url)
        .header("x-token-passcode", passcode)
        .send()
        .await
        .context("Failed to request access token")?
        .error_for_status()
        .context("Token fetcher returned an error")?
        .json::<TokenResponse>()
        .await
        .context("Failed to parse token fetcher response")?;

    Ok(token.access_token)
}
