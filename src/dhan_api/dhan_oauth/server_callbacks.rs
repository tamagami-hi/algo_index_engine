use anyhow::{Result, bail};
use reqwest::{Client, Response, Url};
use serde::de::DeserializeOwned;
use std::time::Duration;
use time::{
    OffsetDateTime, PrimitiveDateTime, UtcOffset, format_description::well_known::Rfc3339,
    macros::format_description,
};

use super::types::{
    Config, ConsentResponse, DhanSession, EXPIRY_MARGIN_SECONDS, TokenResponse, is_valid_token,
};

pub(super) const AUTH_BASE: &str = "https://auth.dhan.co";
const HTTP_TIMEOUT_SECONDS: u64 = 20;
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

fn http_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECONDS))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| anyhow::anyhow!("Could not initialize Dhan authentication client"))
}

async fn decode<T: DeserializeOwned>(response: Result<Response, reqwest::Error>) -> Result<T> {
    let mut response =
        response.map_err(|_| anyhow::anyhow!("Dhan authentication request failed"))?;
    if !response.status().is_success() {
        bail!(
            "Dhan authentication returned HTTP {}",
            response.status().as_u16()
        );
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("Could not read Dhan authentication response"))?
    {
        if chunk.len() > MAX_RESPONSE_BYTES.saturating_sub(body.len()) {
            bail!("Dhan authentication response exceeded the size limit");
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body)
        .map_err(|_| anyhow::anyhow!("Dhan authentication returned an invalid response"))
}

pub(super) async fn generate_consent(config: &Config, base: &str) -> Result<String> {
    let response: ConsentResponse = decode(
        http_client()?
            .post(format!("{base}/app/generate-consent"))
            .header("app_id", &config.api_key)
            .header("app_secret", &config.api_secret)
            .query(&[("client_id", &config.client_id)])
            .send()
            .await,
    )
    .await?;
    if response.status != "success"
        || response.consent_status != "GENERATED"
        || response.consent_id.trim().is_empty()
    {
        bail!("Dhan did not generate a valid login consent");
    }
    let mut url = Url::parse(&format!("{AUTH_BASE}/login/consentApp-login"))
        .map_err(|_| anyhow::anyhow!("Invalid Dhan login endpoint"))?;
    url.query_pairs_mut()
        .append_pair("consentAppId", &response.consent_id);
    Ok(url.into())
}

pub(super) async fn exchange_token(
    config: &Config,
    token_id: &str,
    base: &str,
) -> Result<DhanSession> {
    if token_id.trim().is_empty() || token_id.len() > 4096 || token_id.chars().any(char::is_control)
    {
        bail!("Dhan callback tokenId is missing or invalid");
    }
    let response: TokenResponse = decode(
        http_client()?
            .get(format!("{base}/app/consumeApp-consent"))
            .header("app_id", &config.api_key)
            .header("app_secret", &config.api_secret)
            .query(&[("tokenId", token_id)])
            .send()
            .await,
    )
    .await?;
    validate_session(config, response, OffsetDateTime::now_utc().unix_timestamp())
}

fn validate_session(config: &Config, response: TokenResponse, now: i64) -> Result<DhanSession> {
    if response.client_id != config.client_id {
        bail!("Dhan authenticated a different client ID");
    }
    if !is_valid_token(&response.access_token) {
        bail!("Dhan returned an invalid access token");
    }
    let expires_at = parse_expiry(&response.expiry_time)?;
    if expires_at <= now.saturating_add(EXPIRY_MARGIN_SECONDS) {
        bail!("Dhan returned an expired or nearly expired session");
    }
    Ok(DhanSession {
        client_id: response.client_id,
        api_key: config.api_key.clone(),
        access_token: response.access_token,
        expires_at,
    })
}

fn parse_expiry(value: &str) -> Result<i64> {
    if let Ok(timestamp) = OffsetDateTime::parse(value, &Rfc3339) {
        return Ok(timestamp.unix_timestamp());
    }
    let seconds = format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]");
    let fractional =
        format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond]");
    let timestamp = PrimitiveDateTime::parse(value, seconds)
        .or_else(|_| PrimitiveDateTime::parse(value, fractional))
        .map_err(|_| anyhow::anyhow!("Dhan returned an invalid expiryTime"))?;
    let ist = UtcOffset::from_hms(5, 30, 0).map_err(|_| anyhow::anyhow!("Invalid IST offset"))?;
    Ok(timestamp.assume_offset(ist).unix_timestamp())
}

#[cfg(test)]
#[path = "../../../tests/dhan_api/dhan_oauth/server_callbacks.rs"]
mod tests;
