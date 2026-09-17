use std::path::PathBuf;

use anyhow::{Context, Result};

const ENV_FILE_NAME: &str = ".env";
const HOME_VARIABLE: &str = "BLACKBOX_HOME";

pub(crate) fn home() -> PathBuf {
    std::env::var_os(HOME_VARIABLE)
        .filter(|value| !value.is_empty())
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}

pub(crate) fn data_path(relative: &str) -> PathBuf {
    home().join(relative)
}

pub(crate) fn load_env() -> Result<()> {
    let env_path = home().join(ENV_FILE_NAME);

    if !env_path.exists() {
        return Ok(());
    }

    dotenv::from_path(&env_path)
        .with_context(|| format!("Failed to load environment file: {}", env_path.display()))?;

    Ok(())
}

#[cfg(test)]
#[path = "../tests/support/home.rs"]
pub(crate) mod sandbox;
