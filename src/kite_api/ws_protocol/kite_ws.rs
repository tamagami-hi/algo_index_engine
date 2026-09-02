use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const KITE_WEBSOCKET_URL: &str = "wss://ws.kite.trade/";

pub(crate) async fn ws_kite_connection(api_key: &str, access_token: &str) -> Result<()> {
    let ws_url = format!("{KITE_WEBSOCKET_URL}?api_key={api_key}&access_token={access_token}");

    println!("Connecting to Kite WebSocket...");

    let (mut ws_stream, response) = connect_async(&ws_url)
        .await
        .context("Failed to connect to Kite WebSocket")?;

    println!("Connected successfully: HTTP {}", response.status());

    while let Some(message) = ws_stream.next().await {
        match message.context("Failed to read Kite WebSocket message")? {
            Message::Binary(data) if data.len() == 1 => {}
            Message::Binary(data) => {
                println!("Received {} binary bytes", data.len());
            }
            Message::Text(text) => {
                println!("Text message: {text}");
            }
            Message::Ping(data) => {
                ws_stream
                    .send(Message::Pong(data))
                    .await
                    .context("Failed to reply to Kite WebSocket ping")?;
            }
            Message::Close(frame) => {
                println!("WebSocket closed: {frame:?}");
                break;
            }
            Message::Pong(_) | Message::Frame(_) => {}
        }
    }

    Ok(())
}
