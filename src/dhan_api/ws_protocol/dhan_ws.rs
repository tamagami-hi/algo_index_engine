use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const DHAN_WEBSOCKET_URL: &str = "wss://api-feed.dhan.co/";

pub(crate) async fn ws_dhan_connection(client_id: &str, access_token: &str) -> Result<()> {
    let ws_url = format!(
        "{DHAN_WEBSOCKET_URL}?version=2&token={access_token}&clientId={client_id}&authType=2"
    );

    println!("Connecting to Dhan WebSocket...");

    let (mut ws_stream, response) = connect_async(&ws_url)
        .await
        .context("Failed to connect to Dhan WebSocket")?;

    println!("Connected successfully: HTTP {}", response.status());

    while let Some(message) = ws_stream.next().await {
        match message.context("Failed to read Dhan WebSocket message")? {
            Message::Binary(data) => {
                println!("Received {} Dhan binary bytes", data.len());
            }
            Message::Text(text) => {
                println!("Dhan text message: {text}");
            }
            Message::Ping(data) => {
                ws_stream
                    .send(Message::Pong(data))
                    .await
                    .context("Failed to reply to Dhan WebSocket ping")?;
            }
            Message::Close(frame) => {
                println!("Dhan WebSocket closed: {frame:?}");
                break;
            }
            Message::Pong(_) | Message::Frame(_) => {}
        }
    }

    Ok(())
}
