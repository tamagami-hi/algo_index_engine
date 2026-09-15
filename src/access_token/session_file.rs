//! The on-disk Dhan access token, at `data/sessions/dhan_access_token.json`.
//!
//! Owns both directions so the path and the JSON shape are defined once: a fresh token
//! is written here after every successful fetch, and read back when the token route is
//! unreachable.
//!
//! The token is stored raw and unencrypted, so the file is created with owner-only
//! permissions and `data/sessions/` is gitignored.
//!
//! `expires_at` is stored alongside it because a cached token has to be checked against
//! its real expiry, not against the file's timestamp. A file mtime says when we wrote
//! the token, which is not when Dhan will stop accepting it — and rewriting the file
//! would reset it. Recording the expiry makes the cache self-contained: it can be
//! judged without a network call and without trusting the filesystem.
//!
//! The token route's reply is also kept VERBATIM under `response`, so the file explains
//! itself. Reading it answers what the route said, what was derived from it, and when —
//! without needing this source to interpret. Note that means every field the route
//! sends lands on disk, including the client name and UCC when pointed at Dhan
//! directly, which is part of why the file is owner-only and gitignored.

use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use super::expiry::{
    Expiry, ExpirySource, TOKEN_VALIDITY_SECONDS, format_ist, jwt_expiry,
};

const SESSION_DIRECTORY: &str = "data/sessions";
const SESSION_FILE_NAME: &str = "dhan_access_token.json";

/// The session file's contents. One definition for reading and writing, so the two
/// cannot drift apart.
///
/// Field order is the order they serialise in, chosen so the file reads top to bottom:
/// the token, when it dies, where that was learned, when it was fetched, then the raw
/// reply it all came from.
#[derive(Debug, Deserialize, Serialize)]
struct SessionFile {
    access_token: String,
    /// Unix seconds. Optional so a file written before expiry tracking still loads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<i64>,
    /// The same instant in IST, for reading by eye.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at_ist: Option<String>,
    /// Whether the expiry was stated, decoded from the JWT, or assumed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expiry_source: Option<ExpirySource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fetched_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fetched_at_ist: Option<String>,
    /// The token route's reply exactly as received, so nothing it said is lost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    response: Option<serde_json::Value>,
}

/// Everything worth recording about a freshly fetched token.
#[derive(Debug)]
pub(crate) struct SessionRecord<'token> {
    pub(crate) access_token: &'token str,
    pub(crate) expiry: Expiry,
    pub(crate) fetched_at: i64,
    /// The route's reply, verbatim.
    pub(crate) response: Option<serde_json::Value>,
}

/// A token recovered from disk, with its expiry resolved.
#[derive(Debug)]
pub(crate) struct CachedToken {
    pub(crate) token: String,
    pub(crate) expiry: Expiry,
}

fn session_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(SESSION_DIRECTORY)
        .join(SESSION_FILE_NAME)
}

/// Write the token, its resolved expiry and the raw reply, replacing any previous one.
pub(crate) async fn save_token(record: SessionRecord<'_>) -> Result<PathBuf> {
    let path = session_path();
    let directory = path
        .parent()
        .context("Session file path has no parent directory")?;

    tokio::fs::create_dir_all(directory)
        .await
        .with_context(|| format!("Failed to create session directory: {}", directory.display()))?;

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

/// Read the cached token, or `Ok(None)` when no session file has been written yet.
///
/// A missing file is a normal first-run state, not an error, so it is distinguished
/// from a file that exists but cannot be read or parsed — that one is a real fault and
/// is reported.
///
/// Expiry is recovered in the same precedence order used when the token was fetched:
/// the recorded value, then the token's own JWT claim, then 24 hours from the file's
/// mtime. The last is a genuine last resort for a file written before expiry was
/// tracked; it is the only case where the filesystem timestamp is consulted at all.
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

/// When the file was last written, as Unix seconds, or 0 if unknowable.
///
/// 0 makes an undatable legacy file resolve to an expiry in 1970, i.e. expired, which
/// is the safe direction: it forces a fresh fetch instead of shipping a token of
/// unknown age to the feed.
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
