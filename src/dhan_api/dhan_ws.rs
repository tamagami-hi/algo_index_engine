use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tokio_util::sync::CancellationToken;

use super::feed::{Packet, decode_frame};
use super::instruments::{Catalog, Pool, Subscription};
use crate::server::state::EngineState;

const DHAN_WEBSOCKET_URL: &str = "wss://api-feed.dhan.co/";
const SUBSCRIBE_FULL: u32 = 21;

fn subscribe_message(batch: &[Subscription]) -> String {
    let instruments: Vec<serde_json::Value> = batch
        .iter()
        .map(|instrument| {
            serde_json::json!({
                "ExchangeSegment": instrument.segment.as_str(),
                "SecurityId": instrument.security_id,
            })
        })
        .collect();

    serde_json::json!({
        "RequestCode": SUBSCRIBE_FULL,
        "InstrumentCount": instruments.len(),
        "InstrumentList": instruments,
    })
    .to_string()
}

async fn subscribe_pool<S>(socket: &mut S, pool: &Pool) -> Result<usize>
where
    S: SinkExt<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    let mut sent = 0;
    for batch in pool.messages() {
        socket
            .send(Message::Text(subscribe_message(batch).into()))
            .await
            .with_context(|| format!("failed to subscribe a {} batch", pool.label))?;
        sent += batch.len();
    }
    Ok(sent)
}

pub(crate) async fn ws_dhan_connection(
    client_id: &str,
    access_token: &str,
    catalog: &Catalog,
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

    let mut subscribed = 0;
    for pool in catalog.pools() {
        subscribed += subscribe_pool(&mut ws_stream, pool).await?;
    }
    println!(
        "subscribed {subscribed} instruments in full mode across {} messages",
        catalog.message_count()
    );
    state.feed_subscribed(subscribed);

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
                let messages = decode_frame(&data);
                if messages.is_empty() && !data.is_empty() {
                    state.feed_undecodable(data.len());
                    continue;
                }
                for message in &messages {
                    if let Packet::Disconnect { reason } = message.packet {
                        println!("Dhan feed sent disconnect reason {reason}");
                    }
                    state.feed_packet(message);
                }
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

#[cfg(test)]
#[path = "../../tests/dhan_api/dhan_ws.rs"]
mod tests;
