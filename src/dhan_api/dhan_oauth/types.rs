use super::super::dhan_auth::is_valid_token;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

pub(super) const EXPIRY_MARGIN_SECONDS: i64 = 60;

/// Credentials deliberately have no Debug implementation.
pub(crate) struct Config {
    pub(super) client_id: String,
    pub(super) api_key: String,
    pub(super) api_secret: String,
}

impl Config {
    pub(crate) fn new(client_id: &str, api_key: &str, api_secret: &str) -> Result<Self> {
        for (name, value) in [
            ("DHAN_CLIENT_ID", client_id),
            ("DHAN_API_KEY", api_key),
            ("DHAN_API_SECRET", api_secret),
        ] {
            if !is_valid_token(value) {
                bail!("{name} must contain nonempty printable ASCII without whitespace");
            }
        }
        Ok(Self {
            client_id: client_id.into(),
            api_key: api_key.into(),
            api_secret: api_secret.into(),
        })
    }

    pub(crate) fn from_env() -> Result<Self> {
        let client_id = std::env::var("DHAN_CLIENT_ID").context("Missing DHAN_CLIENT_ID")?;
        let api_key = std::env::var("DHAN_API_KEY").context("Missing DHAN_API_KEY")?;
        let api_secret = std::env::var("DHAN_API_SECRET").context("Missing DHAN_API_SECRET")?;
        Self::new(&client_id, &api_key, &api_secret)
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct DhanSession {
    pub(super) client_id: String,
    pub(super) api_key: String,
    pub(super) access_token: String,
    pub(super) expires_at: i64,
}

impl From<DhanSession> for super::DhanCredentials {
    fn from(session: DhanSession) -> Self {
        Self {
            client_id: session.client_id,
            api_key: session.api_key,
            access_token: session.access_token,
        }
    }
}

#[derive(Deserialize)]
pub(super) struct ConsentResponse {
    #[serde(rename = "consentAppId")]
    pub(super) consent_id: String,
    pub(super) status: String,
    #[serde(rename = "consentAppStatus")]
    pub(super) consent_status: String,
}

#[derive(Deserialize)]
pub(super) struct TokenResponse {
    #[serde(rename = "dhanClientId")]
    pub(super) client_id: String,
    #[serde(rename = "accessToken")]
    pub(super) access_token: String,
    #[serde(rename = "expiryTime")]
    pub(super) expiry_time: String,
}

#[cfg(test)]
#[path = "../../../tests/dhan_api/dhan_oauth/types.rs"]
mod tests;
