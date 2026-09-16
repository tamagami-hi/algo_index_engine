use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use super::expiry::{Expiry, ExpirySource, TOKEN_VALIDITY_SECONDS, format_ist, jwt_expiry};

const SESSION_DIRECTORY: &str = "data/sessions";
const SESSION_FILE_NAME: &str = "dhan_access_token.json";

#[derive(Debug, Deserialize, Serialize)]
struct SessionFile {
    access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at_ist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expiry_source: Option<ExpirySource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fetched_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fetched_at_ist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    response: Option<serde_json::Value>,
}

#[derive(Debug)]
pub(crate) struct SessionRecord<'token> {
    pub(crate) access_token: &'token str,
    pub(crate) expiry: Expiry,
    pub(crate) fetched_at: i64,
    pub(crate) response: Option<serde_json::Value>,
}

#[derive(Debug)]
pub(crate) struct CachedToken {
    pub(crate) token: String,
    pub(crate) expiry: Expiry,
}

fn session_path() -> PathBuf {
    crate::config::data_path(SESSION_DIRECTORY).join(SESSION_FILE_NAME)
}

pub(crate) async fn save_token(record: SessionRecord<'_>) -> Result<PathBuf> {
    let path = session_path();
    let directory = path
        .parent()
        .context("Session file path has no parent directory")?;

    tokio::fs::create_dir_all(directory)
        .await
        .with_context(|| {
            format!(
                "Failed to create session directory: {}",
                directory.display()
            )
        })?;

    let session = SessionFile {
        access_token: record.access_token.to_owned(),
        expires_at: Some(record.expiry.at_unix_seconds),
        expires_at_ist: Some(format_ist(record.expiry.at_unix_seconds)),
        expiry_source: Some(record.expiry.source),
        fetched_at: Some(record.fetched_at),
        fetched_at_ist: Some(format_ist(record.fetched_at)),
        response: record.response,
    };
    let bytes =
        serde_json::to_vec_pretty(&session).context("Failed to serialize Dhan session file")?;

    write_private(&path, &bytes)
        .await
        .with_context(|| format!("Failed to write session file: {}", path.display()))?;

    Ok(path)
}

pub(crate) async fn cached_token() -> Result<Option<CachedToken>> {
    let path = session_path();

    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Failed to read session file: {}", path.display()));
        }
    };

    let session: SessionFile = serde_json::from_slice(&bytes)
        .with_context(|| format!("Failed to parse session file: {}", path.display()))?;
    let token = session.access_token.trim().to_owned();
    if token.is_empty() {
        anyhow::bail!(
            "Session file holds an empty access token: {}",
            path.display()
        );
    }

    let expiry = match session.expires_at {
        Some(at_unix_seconds) => Expiry {
            at_unix_seconds,
            source: session.expiry_source.unwrap_or(ExpirySource::Stated),
        },
        None => match jwt_expiry(&token) {
            Some(at_unix_seconds) => Expiry {
                at_unix_seconds,
                source: ExpirySource::JwtClaim,
            },
            None => Expiry {
                at_unix_seconds: written_at(&path).await + TOKEN_VALIDITY_SECONDS,
                source: ExpirySource::Assumed,
            },
        },
    };

    Ok(Some(CachedToken { token, expiry }))
}

async fn written_at(path: &Path) -> i64 {
    let Ok(metadata) = tokio::fs::metadata(path).await else {
        return 0;
    };
    let Ok(modified) = metadata.modified() else {
        return 0;
    };
    modified
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs() as i64
}

async fn write_private(path: &Path, contents: &[u8]) -> Result<()> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);

    #[cfg(unix)]
    {
        options.mode(0o600);
    }

    let mut file = options.open(path).await?;
    file.write_all(contents).await?;
    file.flush().await?;

    restrict_existing(path).await
}

#[cfg(unix)]
async fn restrict_existing(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await?;
    Ok(())
}

#[cfg(not(unix))]
async fn restrict_existing(_path: &Path) -> Result<()> {
    Ok(())
}
