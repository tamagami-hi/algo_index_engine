use super::super::dhan_auth::is_valid_token;
use super::types::{Config, DhanSession, EXPIRY_MARGIN_SECONDS};
use anyhow::{Context, Result, bail};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_SESSION_BYTES: u64 = 64 * 1024;

pub(super) fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/sessions/dhan_oauth.json")
}

pub(super) fn save(path: &Path, session: &DhanSession) -> Result<()> {
    reject_symlinks(path)?;
    let parent = path
        .parent()
        .context("OAuth session path has no directory")?;
    fs::create_dir_all(parent).context("Cannot create OAuth session directory")?;
    private_permissions(parent, 0o700)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .context("Cannot create private OAuth session file")?;
    private_permissions(temporary.path(), 0o600)?;
    let bytes = serde_json::to_vec(session).context("Cannot encode OAuth session")?;
    temporary
        .write_all(&bytes)
        .context("Cannot write OAuth session")?;
    temporary
        .as_file()
        .sync_all()
        .context("Cannot flush OAuth session")?;
    temporary
        .persist(path)
        .map_err(|_| anyhow::anyhow!("Cannot replace OAuth session"))?;
    Ok(())
}

pub(super) fn load(path: &Path, config: &Config, now: i64) -> Result<Option<DhanSession>> {
    reject_symlinks(path)?;
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => bail!("Cannot read OAuth session; run cargo run -- --dhan-login"),
    };
    let mut bytes = Vec::new();
    file.take(MAX_SESSION_BYTES + 1)
        .read_to_end(&mut bytes)
        .context("Cannot read OAuth session")?;
    if bytes.len() as u64 > MAX_SESSION_BYTES {
        bail!("OAuth session file is too large");
    }
    let session: DhanSession = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("Invalid OAuth session; run cargo run -- --dhan-login"))?;
    let is_valid = is_valid_token(&session.access_token)
        && session.client_id == config.client_id
        && session.api_key == config.api_key
        && session.expires_at > now.saturating_add(EXPIRY_MARGIN_SECONDS);
    Ok(is_valid.then_some(session))
}

fn reject_symlinks(path: &Path) -> Result<()> {
    for ancestor in path
        .ancestors()
        .filter(|entry| !entry.as_os_str().is_empty())
    {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!("OAuth session path must not contain symlinks")
            }
            Ok(metadata) if ancestor == path && !metadata.is_file() => {
                bail!("OAuth session must be a regular file")
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => bail!("Cannot inspect OAuth session path"),
        }
    }
    Ok(())
}

#[cfg(unix)]
fn private_permissions(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .context("Cannot restrict OAuth session permissions")
}

#[cfg(not(unix))]
fn private_permissions(_path: &Path, _mode: u32) -> Result<()> {
    bail!("OAuth session persistence currently requires Unix file permissions")
}

#[cfg(test)]
#[path = "../../../tests/dhan_api/dhan_oauth/session.rs"]
mod tests;
