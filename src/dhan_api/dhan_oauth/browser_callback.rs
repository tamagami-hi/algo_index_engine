use super::super::dhan_auth::is_valid_token;
use anyhow::{Context, Result, bail};
use axum::{
    Router,
    extract::{RawQuery, State},
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use reqwest::Url;
use std::{
    future::IntoFuture,
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use tokio::{
    net::TcpListener,
    sync::{mpsc, oneshot},
};

const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CALLBACK_BYTES: usize = 8192;

pub(super) struct CallbackListener {
    listener: TcpListener,
    redirect: Url,
}

#[derive(Clone)]
struct CallbackState {
    redirect: Url,
    sender: mpsc::Sender<String>,
}

pub(super) async fn bind(redirect: &str) -> Result<CallbackListener> {
    let url = Url::parse(redirect).map_err(|_| anyhow::anyhow!("Invalid Dhan redirect URL"))?;
    let host = url.host_str().unwrap_or_default().trim_matches(['[', ']']);
    let address: IpAddr = host.parse().map_err(|_| {
        anyhow::anyhow!("Automatic Dhan callback requires a numeric loopback address")
    })?;
    if url.scheme() != "http"
        || !matches!(url.host_str(), Some("127.0.0.1" | "[::1]"))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.path().contains(['{', '}'])
        || url.query_pairs().any(|(key, _)| key == "tokenId")
    {
        bail!(
            "Automatic Dhan callback requires an HTTP loopback URL without credentials, fragments or tokenId"
        );
    }
    let port = url
        .port()
        .filter(|port| *port != 0)
        .context("Automatic Dhan callback requires an explicit nonzero port")?;
    let listener = TcpListener::bind(SocketAddr::new(address, port)).await
        .context("Cannot bind Dhan callback port; check DHAN_REDIRECT_URL and close conflicting services")?;
    Ok(CallbackListener {
        listener,
        redirect: url,
    })
}

impl CallbackListener {
    pub(super) async fn receive(self) -> Result<String> {
        let (sender, mut receiver) = mpsc::channel(1);
        let path = self.redirect.path().to_owned();
        let alternate = if path.ends_with('/') {
            path.trim_end_matches('/').to_owned()
        } else {
            format!("{path}/")
        };
        let app = Router::new().route(&path, get(handle_callback));
        let app = if alternate.is_empty() || alternate == path {
            app
        } else {
            app.route(&alternate, get(handle_callback))
        };
        let app = app.with_state(CallbackState {
            redirect: self.redirect,
            sender,
        });
        let (shutdown, finished) = oneshot::channel::<()>();
        let server = axum::serve(self.listener, app)
            .with_graceful_shutdown(async {
                let _ = finished.await;
            })
            .into_future();
        tokio::pin!(server);
        let result = tokio::select! {
            callback = tokio::time::timeout(LOGIN_TIMEOUT, receiver.recv()) => {
                callback.context("Dhan browser login timed out after five minutes")
                    .and_then(|value| value.context("Dhan callback listener closed unexpectedly"))
            },
            served = &mut server => {
                served.context("Dhan callback server failed")?;
                bail!("Dhan callback server stopped before login completed");
            }
        };
        let _ = shutdown.send(());
        tokio::time::timeout(SHUTDOWN_TIMEOUT, &mut server)
            .await
            .context("Dhan callback server shutdown timed out")?
            .context("Dhan callback server failed")?;
        result
    }
}

async fn handle_callback(
    State(state): State<CallbackState>,
    method: Method,
    RawQuery(query): RawQuery,
) -> Response {
    if method != Method::GET {
        return browser_response(
            StatusCode::METHOD_NOT_ALLOWED,
            "Only GET callbacks are accepted.",
        );
    }
    let Ok(token_id) = callback_token(&query.unwrap_or_default(), &state.redirect) else {
        return browser_response(
            StatusCode::BAD_REQUEST,
            "Invalid Dhan callback. Please finish login in the original browser tab.",
        );
    };
    if state.sender.try_send(token_id).is_err() {
        return browser_response(
            StatusCode::CONFLICT,
            "A Dhan callback has already been received.",
        );
    }
    browser_response(
        StatusCode::OK,
        "Dhan login received. You can close this tab and return to the terminal.",
    )
}

pub(super) fn callback_token(query: &str, redirect: &Url) -> Result<String> {
    let prefix = redirect.as_str().split('?').next().unwrap_or_default();
    if prefix.len() + 1 + query.len() > MAX_CALLBACK_BYTES {
        bail!("Callback URL is too long");
    }
    let mut actual = redirect.clone();
    actual.set_query(Some(query));
    let tokens: Vec<_> = actual
        .query_pairs()
        .filter(|(key, _)| key == "tokenId")
        .collect();
    if tokens.len() != 1 || !is_valid_token(&tokens[0].1) {
        bail!("Callback must contain exactly one nonempty tokenId");
    }
    let supplied: Vec<_> = actual
        .query_pairs()
        .filter(|(key, _)| key != "tokenId")
        .collect();
    let configured: Vec<_> = redirect.query_pairs().collect();
    if supplied.len() != configured.len()
        || configured.iter().any(|pair| {
            supplied.iter().filter(|other| *other == pair).count()
                != configured.iter().filter(|other| *other == pair).count()
        })
    {
        bail!("Callback query does not match DHAN_REDIRECT_URL");
    }
    Ok(tokens[0].1.to_string())
}

pub(super) fn browser_response(status: StatusCode, message: &'static str) -> Response {
    (
        status,
        [
            ("cache-control", "no-store"),
            ("referrer-policy", "no-referrer"),
            ("x-content-type-options", "nosniff"),
        ],
        message,
    )
        .into_response()
}

#[cfg(test)]
#[path = "../../../tests/dhan_api/dhan_oauth/browser_callback.rs"]
mod tests;
