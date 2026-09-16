use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use anyhow::{Context, Result, bail};

use super::strategy::Strategy;

/// Serialises read-modify-write of the active set. Activating is read, insert,
/// write; two of those interleaved lose one edit. Held only across file work,
/// never across an await.
static STORE: Mutex<()> = Mutex::new(());

fn guard() -> MutexGuard<'static, ()> {
    STORE.lock().unwrap_or_else(|error| {
        STORE.clear_poison();
        error.into_inner()
    })
}

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

    // A temp name unique per write. A shared one lets one writer's rename publish
    // another writer's bytes, and leaves the loser renaming a file that is gone.
    let unique = format!(
        "{}.{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("strategy"),
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos())
    );
    let temporary = parent.join(unique);
    std::fs::write(&temporary, body)
        .with_context(|| format!("cannot write {}", temporary.display()))?;
    std::fs::rename(&temporary, path)
        .with_context(|| format!("cannot replace {}", path.display()))?;
    Ok(())
}

pub(crate) fn save(strategy: &Strategy) -> Result<()> {
    let _lock = guard();
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

/// A saved file that could not be read back. Reported rather than dropped: a
/// strategy silently vanishing from the list is worse than one showing as broken.
#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct Unreadable {
    pub(crate) file: String,
    pub(crate) problem: String,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub(crate) struct Listing {
    pub(crate) strategies: Vec<Strategy>,
    pub(crate) unreadable: Vec<Unreadable>,
}

pub(crate) fn list() -> Listing {
    let Ok(entries) = std::fs::read_dir(directory()) else {
        return Listing::default();
    };

    let mut listing = Listing::default();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        if path.file_name().is_some_and(|name| name == "active.json") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("?")
            .to_owned();

        match std::fs::read_to_string(&path)
            .map_err(|error| error.to_string())
            .and_then(|body| serde_json::from_str::<Strategy>(&body).map_err(|e| e.to_string()))
        {
            Ok(strategy) => listing.strategies.push(strategy),
            Err(problem) => listing.unreadable.push(Unreadable {
                file: name,
                problem,
            }),
        }
    }

    listing.strategies.sort_by(|left, right| left.id.cmp(&right.id));
    listing.unreadable.sort_by(|left, right| left.file.cmp(&right.file));
    listing
}

pub(crate) fn remove(id: &str) -> Result<()> {
    let _lock = guard();
    let path = definition_path(id);
    std::fs::remove_file(&path).with_context(|| format!("cannot remove {}", path.display()))?;
    let mut ids = read_active();
    if ids.remove(id) {
        write_active(&ids)?;
    }
    Ok(())
}

fn read_active() -> BTreeSet<String> {
    std::fs::read_to_string(active_path())
        .ok()
        .and_then(|body| serde_json::from_str::<BTreeSet<String>>(&body).ok())
        .unwrap_or_default()
}

fn write_active(ids: &BTreeSet<String>) -> Result<()> {
    let body = serde_json::to_string_pretty(ids).context("cannot encode the active set")?;
    write_atomically(&active_path(), &body)
}

pub(crate) fn active() -> BTreeSet<String> {
    let _lock = guard();
    read_active()
}

pub(crate) fn activate(id: &str) -> Result<BTreeSet<String>> {
    let _lock = guard();
    load(id).with_context(|| format!("cannot activate unknown strategy {id}"))?;
    let mut ids = read_active();
    ids.insert(id.to_owned());
    write_active(&ids)?;
    Ok(ids)
}

pub(crate) fn deactivate(id: &str) -> Result<BTreeSet<String>> {
    let _lock = guard();
    let mut ids = read_active();
    ids.remove(id);
    write_active(&ids)?;
    Ok(ids)
}

#[cfg(test)]
#[path = "../../tests/risk_engine/store.rs"]
mod tests;
