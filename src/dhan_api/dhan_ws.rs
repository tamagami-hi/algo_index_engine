use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tokio_util::sync::CancellationToken;

use super::feed::{Packet, decode_frame};
use super::instruments::{Catalog, Pool, Subscription, ist_today};
use crate::server::state::EngineState;

const DHAN_WEBSOCKET_URL: &str = "wss://api-feed.dhan.co/";
const SUBSCRIBE_FULL: u32 = 21;
const ROLL_CHECK: std::time::Duration = std::time::Duration::from_secs(60);
const SILENCE_CHECK: std::time::Duration = std::time::Duration::from_secs(1);

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
    as_of: &str,
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

    tracing::info!(status = %response.status(), "broker feed connected");
    state.feed_connected();

    let mut subscribed = 0;
    for pool in catalog.pools() {
        subscribed += subscribe_pool(&mut ws_stream, pool).await?;
    }
    tracing::info!(
        instruments = subscribed,
        messages = catalog.message_count(),
        mode = "full",
        "subscribed to the market feed"
    );
    state.feed_subscribed(subscribed);

    let mut cancelled = false;
    let mut roll_check = tokio::time::interval(ROLL_CHECK);
    roll_check.tick().await;
    let mut silence_check = tokio::time::interval(SILENCE_CHECK);
    silence_check.tick().await;

    loop {
        let next = tokio::select! {
            () = shutdown.cancelled() => {
                cancelled = true;
                None
            }
            _ = silence_check.tick() => {
                state.check_feed_silence();
                continue;
            }
            _ = roll_check.tick() => {
                state.check_feed_silence();
                match ist_today() {
                    Ok(today) if today != as_of => {
                        tracing::info!(from = %as_of, to = %today, "trading day rolled; reloading the universe");
                        None
                    }
                    _ => continue,
                }
            }
            message = ws_stream.next() => message,
        };

        let Some(message) = next else {
            break;
        };

        match message.context("Failed to read Dhan WebSocket message")? {
            Message::Binary(data) => {
                let messages = decode_frame(&data);
                for message in &messages {
                    if let Packet::Disconnect { reason } = message.packet {
                        tracing::warn!(reason, "broker feed sent a disconnect");
                    }
                }
                state.apply_frame(data.len(), &messages);
            }
            Message::Text(text) => {
                state.apply_frame(text.len(), &[]);
                tracing::debug!(%text, "broker feed text message");
            }
            Message::Ping(data) => {
                ws_stream
                    .send(Message::Pong(data))
                    .await
                    .context("Failed to reply to Dhan WebSocket ping")?;
            }
            Message::Close(frame) => {
                tracing::warn!(frame = ?frame, "broker feed closed");
                break;
            }
            Message::Pong(_) | Message::Frame(_) => {}
        }
    }

    if cancelled {
        tracing::info!("closing the broker feed");
        let _ = ws_stream.send(Message::Close(None)).await;
    }

    Ok(())
}

#[cfg(test)]
#[path = "../../tests/dhan_api/dhan_ws.rs"]
mod tests;
