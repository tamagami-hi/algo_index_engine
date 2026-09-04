use anyhow::{Context, Result};

#[allow(dead_code)]
pub(crate) struct DhanCredentials {
    pub(crate) client_id: String,
    pub(crate) api_key: String,
    pub(crate) access_token: String,
}

pub(crate) async fn get_dhan_credentials() -> Result<DhanCredentials> {
    
    let api_key = std::env::var("DHAN_API_KEY").context("Missing DHAN_API_KEY environment variable")?;
    let client_id = std::env::var("DHAN_CLIENT_ID").context("Missing DHAN_CLIENT_ID environment variable")?;

    // A token pasted straight from the Dhan dashboard wins, so the feed can run even
    // when the Cal Spread token route has no live Dhan session to hand one out.
    let access_token = match std::env::var("DHAN_ACCESS_TOKEN") {
        Ok(token) if !token.trim().is_empty() => token.trim().to_owned(),
        _ => {
            let url = std::env::var("DHAN_TOKEN_URL")
                .context("Missing DHAN_TOKEN_URL environment variable")?;

            let token = crate::utils::access_token::get_token(&url)
                .await
                .context("Failed to fetch DHAN access token")?;

            let session_path = crate::utils::access_token::save_token(&token)
                .await
                .context("Failed to save DHAN access token session file")?;
            println!("Dhan access token saved to {}", session_path.display());

            token
        }
    };

    Ok(DhanCredentials {
        client_id,
        access_token,
        api_key,
    })
}
