use std::path::PathBuf;

use anyhow::{Context, Result};
use reqwest::Client;

const INSTRUMENT_MASTER_URL: &str = "https://images.dhan.co/api-data/api-scrip-master-detailed.csv";
const INSTRUMENT_DIRECTORY: &str = "data/instruments";
const INSTRUMENT_FILE_NAME: &str = "dhan_instruments.csv";

pub(crate) async fn download_instrument_master() -> Result<PathBuf> {
    let instrument_data = Client::new()
        .get(INSTRUMENT_MASTER_URL)
        .send()
        .await
        .context("Failed to download Dhan instrument master")?
        .error_for_status()
        .context("Dhan instrument master request returned an error")?
        .bytes()
        .await
        .context("Failed to read Dhan instrument master response")?;

    let instrument_directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(INSTRUMENT_DIRECTORY);
    tokio::fs::create_dir_all(&instrument_directory)
        .await
        .with_context(|| {
            format!(
                "Failed to create instrument directory: {}",
                instrument_directory.display()
            )
        })?;

    let instrument_path = instrument_directory.join(INSTRUMENT_FILE_NAME);
    tokio::fs::write(&instrument_path, instrument_data)
        .await
        .with_context(|| {
            format!(
                "Failed to save Dhan instrument master: {}",
                instrument_path.display()
            )
        })?;

    Ok(instrument_path)
}
