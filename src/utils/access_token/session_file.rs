//! The on-disk Dhan access token, at `data/sessions/dhan_access_token.json`.
//!
//! Owns both directions so the path and the JSON shape are defined once: a fresh token
//! is written here after every successful fetch, and read back when the token route is
//! unreachable.
//!
//! The token is stored raw and unencrypted, so the file is created with owner-only
//! permissions and `data/sessions/` is gitignored.

use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

const SESSION_DIRECTORY: &str = "data/sessions";
const SESSION_FILE_NAME: &str = "dhan_access_token.json";

/// The session file's contents. One definition, used for both reading and writing, so
/// the two can never drift apart.
#[derive(Debug, Deserialize, Serialize)]
struct SessionFile {
    access_token: String,
}

/// A token recovered from disk.
#[derive(Debug)]
pub(crate) struct CachedToken {
    pub(crate) token: String,
    /// How long ago the file was written, when the filesystem can say.
    ///
    /// Dhan access tokens expire, so age is the difference between a usable fallback
    /// and one that will be rejected at the feed handshake. Reported rather than
    /// enforced: this module does not decide policy, it just says how old the token is.
    pub(crate) age: Option<Duration>,
}

fn session_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(SESSION_DIRECTORY)
        .join(SESSION_FILE_NAME)
}

/// Write `access_token` to the session file, replacing any previous one.
pub(crate) async fn save_token(access_token: &str) -> Result<PathBuf> {
    let path = session_path();
    let directory = path
        .parent()
        .context("Session file path has no parent directory")?;

    tokio::fs::create_dir_all(directory)
        .await
        .with_context(|| format!("Failed to create session directory: {}", directory.display()))?;

    let session = SessionFile {
        access_token: access_token.to_owned(),
    };
    let bytes =
        serde_json::to_vec_pretty(&session).context("Failed to serialize Dhan session file")?;

    write_private(&path, &bytes)
        .await
        .with_context(|| format!("Failed to write session file: {}", path.display()))?;

    Ok(path)
}

/// Read the cached token, or `Ok(None)` when no session file has been written yet.
///
/// A missing file is a normal first-run state, not an error, so it is distinguished
/// from a file that exists but cannot be read or parsed — that one is a real fault and
/// is reported.
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
        anyhow::bail!("Session file holds an empty access token: {}", path.display());
    }

    Ok(Some(CachedToken {
        token,
        age: file_age(&path).await,
    }))
}

/// How long ago the file was last written, or `None` if the platform cannot say.
async fn file_age(path: &Path) -> Option<Duration> {
    let modified = tokio::fs::metadata(path).await.ok()?.modified().ok()?;
    SystemTime::now().duration_since(modified).ok()
}

/// Creates or truncates the file and writes it, keeping it readable only by its owner
/// rather than chmod-ing after the fact and leaving the token briefly world-readable.
async fn write_private(path: &Path, contents: &[u8]) -> Result<()> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);

    #[cfg(unix)]
    {
        // `mode` is an inherent unix-only method on tokio's OpenOptions.
        options.mode(0o600);
    }

    let mut file = options.open(path).await?;
    file.write_all(contents).await?;
    file.flush().await?;

    // `mode` above only applies while creating, so tighten an already existing file too.
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
