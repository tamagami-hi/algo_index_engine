use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokio::io::AsyncWriteExt;

const SESSION_DIRECTORY: &str = "data/sessions";
const SESSION_FILE_NAME: &str = "dhan_access_token.json";

pub(crate) async fn save_token(access_token: &str) -> Result<PathBuf> {
    let session_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(SESSION_DIRECTORY)
        .join(SESSION_FILE_NAME);
    let session_directory = session_path
        .parent()
        .context("Session file path has no parent directory")?;

    tokio::fs::create_dir_all(session_directory)
        .await
        .with_context(|| {
            format!(
                "Failed to create session directory: {}",
                session_directory.display()
            )
        })?;

    let session = serde_json::json!({ "access_token": access_token });
    let session_bytes =
        serde_json::to_vec_pretty(&session).context("Failed to serialize Dhan session file")?;

    write_private(&session_path, &session_bytes)
        .await
        .with_context(|| format!("Failed to write session file: {}", session_path.display()))?;

    Ok(session_path)
}


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
