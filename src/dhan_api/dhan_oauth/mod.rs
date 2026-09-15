//! Dhan's individual API-key browser consent flow.
mod browser_callback;
mod server_callbacks;
mod session;
mod types;

#[cfg(test)]
#[path = "../../../tests/dhan_api/dhan_oauth/flow_tests.rs"]
mod flow_tests;

use super::dhan_auth::DhanCredentials;
use anyhow::{Context, Result};
use server_callbacks::{AUTH_BASE, exchange_token, generate_consent};
use types::{Config, DhanSession, EXPIRY_MARGIN_SECONDS};

pub(crate) fn saved_token(client_id: &str, api_key: &str) -> Option<String> {
    let session = session::read(&session::path())?;
    if session.client_id != client_id || session.api_key != api_key {
        return None;
    }
    let remaining = session.expires_at - time::OffsetDateTime::now_utc().unix_timestamp();
    (remaining > EXPIRY_MARGIN_SECONDS).then(|| {
        println!(
            "Reusing the saved Dhan login session, {} minutes left.",
            remaining / 60
        );
        session.access_token
    })
}

pub(crate) async fn get_credentials() -> Result<DhanCredentials> {
    let config = Config::from_env()?;
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    if let Some(existing) = session::load(&session::path(), &config, now)? {
        return Ok(existing.into());
    }
    Ok(interactive_login(&config).await?.into())
}

pub(crate) async fn login() -> Result<DhanCredentials> {
    let session = interactive_login(&Config::from_env()?).await?;
    println!("Dhan login complete. The session has been saved.");
    Ok(session.into())
}

async fn interactive_login(config: &Config) -> Result<DhanSession> {
    let redirect = std::env::var("DHAN_REDIRECT_URL").context("Missing DHAN_REDIRECT_URL")?;
    let callback = browser_callback::bind(&redirect).await?;
    let login_url = generate_consent(config, AUTH_BASE).await?;
    println!("Open this Dhan login URL in your browser:\n{login_url}");
    open_browser(&login_url).await;
    println!("Waiting up to five minutes for Dhan login and 2FA in your browser...");
    let token_id = callback.receive().await?;
    let authenticated = exchange_token(config, &token_id, AUTH_BASE).await?;
    session::save(&session::path(), &authenticated)?;
    Ok(authenticated)
}

async fn open_browser(url: &str) {
    // Invoke the OS opener directly: never interpolate an authentication URL into a shell.
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::process::Command::new("xdg-open")
            .arg(url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .status(),
    )
    .await;
    if !matches!(result, Ok(Ok(status)) if status.success()) {
        eprintln!(
            "Could not open a browser automatically. Open the login URL shown above on this machine."
        );
    }
}
