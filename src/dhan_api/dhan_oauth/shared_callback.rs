use super::browser_callback::{browser_response, callback_token};
use anyhow::{Context, Result, bail};
use axum::{
    Router,
    extract::{RawQuery, State},
    http::{Method, StatusCode},
    response::Response,
    routing::get,
};
use reqwest::Url;
use std::{
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::oneshot;

const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

type ActiveAttempt = Arc<Mutex<Option<Attempt>>>;

struct Attempt {
    identity: Arc<()>,
    sender: Option<oneshot::Sender<String>>,
}

#[derive(Clone)]
pub(crate) struct SharedCallback {
    redirect: Url,
    active: ActiveAttempt,
}

pub(crate) struct PendingCallback {
    active: ActiveAttempt,
    identity: Arc<()>,
    receiver: Option<oneshot::Receiver<String>>,
}

impl SharedCallback {
    pub(crate) fn new(redirect: &str) -> Result<Self> {
        let redirect =
            Url::parse(redirect).map_err(|_| anyhow::anyhow!("Invalid Dhan redirect URL"))?;

        // Messages below never echo the input: a redirect can carry credentials or
        // a token, and an error string is the one place they would escape.
        if redirect.scheme() != "http" {
            bail!(
                "Dhan redirect must use http. The callback is served by the backend's own loopback listener, so routing a public HTTPS origin to it needs proxy routing and a matching Dhan app registration, which is a separate change"
            );
        }

        let host = redirect
            .host_str()
            .unwrap_or_default()
            .trim_matches(['[', ']']);
        let is_loopback = host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback());
        if !is_loopback {
            bail!(
                "Dhan redirect host must be a numeric loopback address such as 127.0.0.1 or [::1]. A name is not accepted because resolution could point the browser somewhere else"
            );
        }

        if !matches!(redirect.path(), "/dhan/callback" | "/dhan/callback/") {
            bail!("Dhan redirect path must be /dhan/callback");
        }

        if !redirect.username().is_empty() || redirect.password().is_some() {
            bail!("Dhan redirect must not carry credentials");
        }

        if redirect.fragment().is_some() {
            bail!("Dhan redirect must not carry a fragment");
        }

        if redirect.query_pairs().any(|(key, _)| key == "tokenId") {
            bail!("Dhan redirect must not preset tokenId");
        }

        if redirect.port().is_none_or(|port| port == 0) {
            bail!(
                "Dhan redirect must name an explicit nonzero port, and it must be the port BLACKBOX_HTTP_ADDR publishes"
            );
        }

        Ok(Self {
            redirect,
            active: Arc::new(Mutex::new(None)),
        })
    }

    pub(crate) fn router(&self) -> Router<()> {
        Router::new()
            .route("/dhan/callback", get(handle_callback))
            .route("/dhan/callback/", get(handle_callback))
            .with_state(self.clone())
    }

    pub(crate) fn matches_listener(&self, listener: SocketAddr) -> Result<()> {
        let callback_port = self
            .redirect
            .port()
            .context("Dhan redirect requires an explicit nonzero port")?;
        anyhow::ensure!(
            callback_port == listener.port(),
            "DHAN_REDIRECT_URL must use the same port as BLACKBOX_HTTP_ADDR"
        );
        Ok(())
    }

    pub(crate) fn begin(&self) -> Result<PendingCallback> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| anyhow::anyhow!("Dhan callback state unavailable"))?;
        if active.is_some() {
            bail!("A Dhan browser login is already pending");
        }
        let (sender, receiver) = oneshot::channel();
        let identity = Arc::new(());
        *active = Some(Attempt {
            identity: identity.clone(),
            sender: Some(sender),
        });
        Ok(PendingCallback {
            active: self.active.clone(),
            identity,
            receiver: Some(receiver),
        })
    }
}

impl PendingCallback {
    pub(crate) async fn receive(self) -> Result<String> {
        self.receive_with_timeout(LOGIN_TIMEOUT).await
    }

    async fn receive_with_timeout(mut self, timeout: Duration) -> Result<String> {
        let receiver = self
            .receiver
            .take()
            .context("Dhan callback receiver unavailable")?;
        tokio::time::timeout(timeout, receiver)
            .await
            .context("Dhan browser login timed out after five minutes")?
            .context("Dhan callback receiver closed unexpectedly")
    }
}

impl Drop for PendingCallback {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock()
            && active
                .as_ref()
                .is_some_and(|attempt| Arc::ptr_eq(&attempt.identity, &self.identity))
        {
            *active = None;
        }
    }
}

async fn handle_callback(
    State(state): State<SharedCallback>,
    method: Method,
    RawQuery(query): RawQuery,
) -> Response {
    if method != Method::GET {
        return browser_response(
            StatusCode::METHOD_NOT_ALLOWED,
            "Only GET callbacks are accepted.",
        );
    }
    let Ok(token) = callback_token(&query.unwrap_or_default(), &state.redirect) else {
        return browser_response(
            StatusCode::BAD_REQUEST,
            "Invalid Dhan callback. Please finish login in the original browser tab.",
        );
    };
    let Ok(mut active) = state.active.lock() else {
        return browser_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "Dhan callback is unavailable.",
        );
    };
    let Some(sender) = active.as_mut().and_then(|attempt| attempt.sender.take()) else {
        return browser_response(
            StatusCode::CONFLICT,
            "No Dhan login is awaiting a callback.",
        );
    };
    if sender.send(token).is_err() {
        return browser_response(
            StatusCode::CONFLICT,
            "Dhan login is no longer awaiting a callback.",
        );
    }
    browser_response(
        StatusCode::OK,
        "Dhan login received. You can close this tab and return to the terminal.",
    )
}

#[cfg(test)]
#[path = "../../../tests/dhan_api/dhan_oauth/shared_callback.rs"]
mod tests;
