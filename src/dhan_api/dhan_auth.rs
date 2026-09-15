use anyhow::{Context, Result, bail};

#[allow(dead_code)]
pub(crate) struct DhanCredentials {
    pub(crate) client_id: String,
    pub(crate) api_key: String,
    pub(crate) access_token: String,
}

pub(super) fn is_valid_token(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_graphic())
}

#[derive(Debug, PartialEq, Eq)]
enum AuthMode {
    Web,
    TokenUrl,
    Manual,
}

fn select_mode(mode: Option<&str>, token: Option<&str>) -> Result<AuthMode> {
    match mode.map(str::trim).filter(|value| !value.is_empty()) {
        Some("web" | "oauth") => Ok(AuthMode::Web),
        Some("token_url") => Ok(AuthMode::TokenUrl),
        Some("manual") => Ok(AuthMode::Manual),
        None if token.is_some_and(|value| !value.trim().is_empty()) => Ok(AuthMode::Manual),
        None => Ok(AuthMode::TokenUrl),
        Some(_) => bail!(
            "Invalid DHAN_AUTH_MODE; choose web, token_url, or manual (oauth is an alias for web)"
        ),
    }
}

fn manual_token(token: Option<&str>) -> Result<String> {
    let token = token.unwrap_or_default().trim();
    if !is_valid_token(token) {
        bail!(
            "Manual Dhan authentication requires a nonempty DHAN_ACCESS_TOKEN without internal whitespace"
        );
    }
    Ok(token.to_owned())
}

pub(crate) async fn get_dhan_credentials() -> Result<DhanCredentials> {
    let api_key =
        std::env::var("DHAN_API_KEY").context("Missing DHAN_API_KEY environment variable")?;
    let client_id =
        std::env::var("DHAN_CLIENT_ID").context("Missing DHAN_CLIENT_ID environment variable")?;
    let mode = optional_env("DHAN_AUTH_MODE")?;
    let token = optional_env("DHAN_ACCESS_TOKEN")?;
    let mode = select_mode(mode.as_deref(), token.as_deref())?;

    if mode != AuthMode::Manual
        && let Some(access_token) = saved_token(&client_id, &api_key).await
    {
        return Ok(DhanCredentials {
            client_id,
            api_key,
            access_token,
        });
    }

    let access_token = match mode {
        AuthMode::Web => return super::dhan_oauth::get_credentials().await,
        AuthMode::Manual => manual_token(token.as_deref()),
        AuthMode::TokenUrl => {
            let url = std::env::var("DHAN_TOKEN_URL")
                .context("Token URL authentication requires DHAN_TOKEN_URL")?;
            if url.trim().is_empty() {
                bail!("Token URL authentication requires a nonempty DHAN_TOKEN_URL");
            }
            crate::access_token::get_token(&url)
                .await
                .context("Failed to obtain DHAN access token")
        }
    }?;
    Ok(DhanCredentials {
        client_id,
        api_key,
        access_token,
    })
}

async fn saved_token(client_id: &str, api_key: &str) -> Option<String> {
    match crate::access_token::saved_token().await {
        Some(token) => Some(token),
        None => super::dhan_oauth::saved_token(client_id, api_key),
    }
}

fn optional_env(name: &str) -> Result<Option<String>> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => bail!("{name} must contain valid Unicode text"),
    }
}

#[cfg(test)]
#[path = "../../tests/dhan_api/dhan_auth.rs"]
mod tests;