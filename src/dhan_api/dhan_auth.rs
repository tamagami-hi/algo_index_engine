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
    
    let url = std::env::var("DHAN_TOKEN_URL").context("Missing DHAN_TOKEN_URL environment variable")?;
    
    let access_token = crate::utils::access_token::get_token(&url)
        .await
        .context("Failed to fetch DHAN access token")?;

    Ok(DhanCredentials {
        client_id,
        access_token,
        api_key,
    })
}
