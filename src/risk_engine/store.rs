use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::strategy::Strategy;

const STRATEGY_DIRECTORY: &str = "data/strategies";
const ACTIVE_FILE: &str = "data/strategies/active.json";

fn directory() -> PathBuf {
    crate::config::data_path(STRATEGY_DIRECTORY)
}

fn active_path() -> PathBuf {
    crate::config::data_path(ACTIVE_FILE)
}

fn definition_path(id: &str) -> PathBuf {
    directory().join(format!("{id}.json"))
}

fn write_atomically(path: &Path, body: &str) -> Result<()> {
    let parent = path
        .parent()
        .context("strategy path has no parent directory")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("cannot create {}", parent.display()))?;

    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, body)
        .with_context(|| format!("cannot write {}", temporary.display()))?;
    std::fs::rename(&temporary, path)
        .with_context(|| format!("cannot replace {}", path.display()))?;
    Ok(())
}

pub(crate) fn save(strategy: &Strategy) -> Result<()> {
    if let Err(problem) = strategy.validate() {
        bail!("strategy is not valid: {}", serde_json::to_string(&problem)?);
    }
    let body = serde_json::to_string_pretty(strategy).context("cannot encode strategy")?;
    write_atomically(&definition_path(&strategy.id), &body)
}

pub(crate) fn load(id: &str) -> Result<Strategy> {
    let path = definition_path(id);
    let body = std::fs::read_to_string(&path)
        .with_context(|| format!("no saved strategy at {}", path.display()))?;
    serde_json::from_str(&body).with_context(|| format!("cannot parse {}", path.display()))
}

pub(crate) fn list() -> Vec<Strategy> {
    let Ok(entries) = std::fs::read_dir(directory()) else {
        return Vec::new();
    };

    let mut strategies: Vec<Strategy> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                return None;
            }
            if path.file_name().is_some_and(|name| name == "active.json") {
                return None;
            }
            let body = std::fs::read_to_string(&path).ok()?;
            serde_json::from_str::<Strategy>(&body).ok()
        })
        .collect();

    strategies.sort_by(|left, right| left.id.cmp(&right.id));
    strategies
}

pub(crate) fn remove(id: &str) -> Result<()> {
    let path = definition_path(id);
    std::fs::remove_file(&path).with_context(|| format!("cannot remove {}", path.display()))?;
    let mut active = active();
    if active.remove(id) {
        set_active(&active)?;
    }
    Ok(())
}

pub(crate) fn active() -> BTreeSet<String> {
    std::fs::read_to_string(active_path())
        .ok()
        .and_then(|body| serde_json::from_str::<BTreeSet<String>>(&body).ok())
        .unwrap_or_default()
}

pub(crate) fn set_active(ids: &BTreeSet<String>) -> Result<()> {
    let body = serde_json::to_string_pretty(ids).context("cannot encode the active set")?;
    write_atomically(&active_path(), &body)
}

pub(crate) fn activate(id: &str) -> Result<BTreeSet<String>> {
    load(id).with_context(|| format!("cannot activate unknown strategy {id}"))?;
    let mut ids = active();
    ids.insert(id.to_owned());
    set_active(&ids)?;
    Ok(ids)
}

pub(crate) fn deactivate(id: &str) -> Result<BTreeSet<String>> {
    let mut ids = active();
    ids.remove(id);
    set_active(&ids)?;
    Ok(ids)
}

#[cfg(test)]
#[path = "../../tests/risk_engine/store.rs"]
mod tests;
