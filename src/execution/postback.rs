use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use anyhow::{Context, Result};
use reqwest::Url;

pub(crate) const POSTBACK_PATH: &str = "/dhan/postback";

pub(crate) const POSTBACK_URL_VARIABLE: &str = "DHAN_POSTBACK_URL";

pub(crate) const MAX_POSTBACK_BYTES: usize = 16 * 1024;

pub(crate) const POSTBACK_BODY_LIMIT_BYTES: usize = 64 * 1024;

const _ACCEPTED_BELOW_TRANSPORT: () = assert!(
    MAX_POSTBACK_BYTES < POSTBACK_BODY_LIMIT_BYTES,
    "the handler must see an oversized body to count and log it, so the transport limit has to be the looser of the two"
);

const _TRANSPORT_STAYS_SMALL: () = assert!(
    POSTBACK_BODY_LIMIT_BYTES <= 64 * 1024,
    "an unauthenticated endpoint must not buffer megabytes per request"
);

const JOURNAL_DIRECTORY: &str = "data/execution";
const UNDATED_DAY: &str = "undated";

static JOURNAL: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Stored {
    pub(crate) day: String,
    pub(crate) records: usize,
}

fn guard() -> MutexGuard<'static, ()> {
    JOURNAL.lock().unwrap_or_else(|error| {
        JOURNAL.clear_poison();
        error.into_inner()
    })
}

pub(crate) fn journal_path(day: &str) -> PathBuf {
    crate::config::data_path(&format!("{JOURNAL_DIRECTORY}/postbacks-{day}.jsonl"))
}

pub(crate) fn record(body: &str, source: Option<&str>) -> Result<Stored> {
    let day = crate::dhan_api::instruments::ist_today().unwrap_or_else(|_| UNDATED_DAY.to_owned());
    let path = journal_path(&day);

    let record = serde_json::json!({
        "received_at_ms": crate::server::state::now_millis(),
        "source": source,
        "bytes": body.len(),
        "body": body,
    });
    let mut line = serde_json::to_string(&record).context("cannot encode a postback record")?;
    line.push('\n');

    let _lock = guard();

    let parent = path
        .parent()
        .context("postback journal path has no parent directory")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("cannot create {}", parent.display()))?;

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("cannot open {}", path.display()))?;
    file.write_all(line.as_bytes())
        .with_context(|| format!("cannot append to {}", path.display()))?;
    file.flush()
        .with_context(|| format!("cannot flush {}", path.display()))?;

    Ok(Stored {
        records: journal_records(&path),
        day,
    })
}

fn journal_records(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .map(|body| body.lines().filter(|line| !line.trim().is_empty()).count())
        .unwrap_or(0)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PostbackUrl {
    Unset,
    LocalOnly,
    Reachable,
}

fn is_loopback_host(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<std::net::IpAddr>()
        .is_ok_and(|address| address.is_loopback())
}

pub(crate) fn configured(listener: SocketAddr) -> Result<PostbackUrl> {
    let raw = match std::env::var(POSTBACK_URL_VARIABLE) {
        Ok(value) if !value.trim().is_empty() => value.trim().to_owned(),
        _ => return Ok(PostbackUrl::Unset),
    };

    let url = Url::parse(&raw)
        .map_err(|_| anyhow::anyhow!("{POSTBACK_URL_VARIABLE} is not a valid URL"))?;

    anyhow::ensure!(
        matches!(url.scheme(), "http" | "https"),
        "{POSTBACK_URL_VARIABLE} must use http or https; Dhan posts over HTTP and nothing else can reach the engine"
    );
    anyhow::ensure!(
        url.path() == POSTBACK_PATH,
        "{POSTBACK_URL_VARIABLE} must use the path {POSTBACK_PATH}; the engine serves order postbacks nowhere else, so any other path answers 404 and every order event behind it is lost"
    );
    anyhow::ensure!(
        url.query().is_none() && url.fragment().is_none(),
        "{POSTBACK_URL_VARIABLE} must carry no query or fragment; the route matches on the path alone and a query cannot authenticate a sender Dhan does not sign for"
    );

    if !is_loopback_host(&url) {
        tracing::info!(
            url = %url,
            path = POSTBACK_PATH,
            "order postback URL configured; register this exact URL with the Dhan access token and make sure the edge routes it to the engine"
        );
        return Ok(PostbackUrl::Reachable);
    }

    let port = url.port().with_context(|| {
        format!(
            "{POSTBACK_URL_VARIABLE} needs an explicit port when it names a loopback host, so it can be checked against the listener"
        )
    })?;
    anyhow::ensure!(
        port == listener.port(),
        "{POSTBACK_URL_VARIABLE} must use the same port as the backend listener; postbacks arrive on the port the engine serves, not a second one"
    );
    tracing::warn!(
        url = %url,
        "Dhan does not deliver postbacks to localhost or 127.0.0.1, so this URL only works for local testing; a live account needs a publicly routable URL"
    );
    Ok(PostbackUrl::LocalOnly)
}

#[cfg(test)]
#[path = "../../tests/execution/postback.rs"]
mod tests;
