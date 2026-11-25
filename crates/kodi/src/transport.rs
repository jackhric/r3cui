//! The WebSocket to Kodi's JSON-RPC (port 9090): text frames out in the order sent, text frames in. The socket is
//! pinged while idle, and counts as dead after 25 s without a frame.

use crate::error::{Error, Result};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

const PING_EVERY: Duration = Duration::from_secs(10);
const SILENT_FOR: Duration = Duration::from_secs(25);

/// One open connection: frames to send, and frames received. The receiver ends when the socket does.
pub(crate) struct Connection {
    pub(crate) out: mpsc::UnboundedSender<String>,
    pub(crate) incoming: mpsc::UnboundedReceiver<String>,
}

pub(crate) async fn connect(url: &str) -> Result<Connection> {
    let unreachable = |e: &dyn std::fmt::Display| Error::Unreachable { endpoint: url.into(), reason: e.to_string().into() };
    let (socket, _) = tokio::time::timeout(Duration::from_secs(5), tokio_tungstenite::connect_async(url))
        .await
        .map_err(|_| unreachable(&"no answer in 5 s"))?
        .map_err(|e| unreachable(&e))?;
    let (mut sink, mut stream) = socket.split();
    let (out, mut to_send) = mpsc::unbounded_channel::<String>();
    let (received, incoming) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        let mut ping = tokio::time::interval(PING_EVERY);
        loop {
            tokio::select! {
                frame = to_send.recv() => match frame {
                    Some(text) => if sink.send(Message::Text(text)).await.is_err() { break },
                    None => break,
                },
                _ = ping.tick() => if sink.send(Message::Ping(Vec::new())).await.is_err() { break },
            }
        }
        let _ = sink.close().await;
    });
    tokio::spawn(async move {
        loop {
            match tokio::time::timeout(SILENT_FOR, stream.next()).await {
                Ok(Some(Ok(Message::Text(text)))) => {
                    if received.send(text).is_err() {
                        break;
                    }
                }
                Ok(Some(Ok(Message::Close(_)) | Err(_)) | None) => break,
                Ok(Some(Ok(_))) => {}
                Err(_) => {
                    tracing::info!("kodi: no frame for {SILENT_FOR:?}, closing");
                    break;
                }
            }
        }
    });
    Ok(Connection { out, incoming })
}
