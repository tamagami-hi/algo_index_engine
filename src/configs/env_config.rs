use std::path::PathBuf;

use anyhow::{Context, Result};

const ENV_FILE_NAME: &str = ".env";

fn env_file_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(ENV_FILE_NAME)
}

pub(crate) fn load_env() -> Result<()> {
    let env_path = env_file_path();

    dotenv::from_path(&env_path)
        .with_context(|| format!("Failed to load environment file: {}", env_path.display()))?;

    Ok(())
}
