use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct TokenRequest {
    passcode: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

async fn get_token() -> Result<String> {
    let url =
        std::env::var("KITE_TOKEN_URL").context("Missing KITE_TOKEN_URL environment variable")?;

    let passcode = std::env::var("KITE_TOKEN_PASSCODE")
        .context("Missing KITE_TOKEN_PASSCODE environment variable")?;

    let client = Client::new();

    let response = client
        .post(url)
        .json(&TokenRequest { passcode })
        .send()
        .await
        .context("Failed to get token")?
        .error_for_status()
        .context("Token Fetcher returned an error")?;

    let token = response
        .json::<TokenResponse>()
        .await
        .context("Failed to parse token response")?;

    Ok(token.access_token)
}
