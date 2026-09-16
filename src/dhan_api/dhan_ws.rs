use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tokio_util::sync::CancellationToken;

use crate::server::state::EngineState;

const DHAN_WEBSOCKET_URL: &str = "wss://api-feed.dhan.co/";

pub(crate) async fn ws_dhan_connection(
    client_id: &str,
    access_token: &str,
    state: &EngineState,
    shutdown: &CancellationToken,
) -> Result<()> {
    let ws_url = format!(
        "{DHAN_WEBSOCKET_URL}?version=2&token={access_token}&clientId={client_id}&authType=2"
    );

    let (mut ws_stream, response) = tokio::select! {
        () = shutdown.cancelled() => return Ok(()),
        connected = connect_async(&ws_url) => {
            connected.context("Failed to connect to Dhan WebSocket")?
        }
    };

    println!("Dhan feed connected: HTTP {}", response.status());
    state.feed_connected();

    let mut cancelled = false;
    loop {
        let next = tokio::select! {
            () = shutdown.cancelled() => {
                cancelled = true;
                None
            }
            message = ws_stream.next() => message,
        };

        let Some(message) = next else {
            break;
        };

        match message.context("Failed to read Dhan WebSocket message")? {
            Message::Binary(data) => {
                state.feed_frame(data.len());
            }
            Message::Text(text) => {
                state.feed_frame(text.len());
                println!("Dhan text message: {text}");
            }
            Message::Ping(data) => {
                ws_stream
                    .send(Message::Pong(data))
                    .await
                    .context("Failed to reply to Dhan WebSocket ping")?;
            }
            Message::Close(frame) => {
                println!("Dhan feed closed: {frame:?}");
                break;
            }
            Message::Pong(_) | Message::Frame(_) => {}
        }
    }

    if cancelled {
        println!("closing the Dhan feed");
        let _ = ws_stream.send(Message::Close(None)).await;
    }

    Ok(())
}
