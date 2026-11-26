use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{interval, sleep, Instant};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

const PING_EVERY: Duration = Duration::from_secs(10);
const SILENT_FOR: Duration = Duration::from_secs(25);
const CONNECT_WITHIN: Duration = Duration::from_secs(5);

/// How a connection watches itself.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// A ping goes out this often, so a quiet but healthy peer still answers.
    pub ping_every: Duration,
    /// No frame of any kind (a pong counts) for this long and the peer is taken for dead.
    pub silent_for: Duration,
    pub connect_within: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self { ping_every: PING_EVERY, silent_for: SILENT_FOR, connect_within: CONNECT_WITHIN }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{url}: {reason}")]
pub struct ConnectError {
    pub url: String,
    pub reason: String,
}

/// One open connection: text frames to send, in the order given, and text frames received. `incoming` ends when the
/// socket does, however it ended; dropping `incoming`, or every clone of `out`, closes the socket.
pub struct Connection {
    pub out: mpsc::UnboundedSender<String>,
    pub incoming: mpsc::UnboundedReceiver<String>,
}

pub async fn connect(request: impl IntoClientRequest) -> Result<Connection, ConnectError> {
    connect_with(request, Timing::default()).await
}

pub async fn connect_with(request: impl IntoClientRequest, timing: Timing) -> Result<Connection, ConnectError> {
    let request = request.into_client_request().map_err(|e| ConnectError { url: String::new(), reason: e.to_string() })?;
    let url = request.uri().to_string();
    let failed = |reason: String| ConnectError { url: url.clone(), reason };
    let (socket, _) = tokio::time::timeout(timing.connect_within, tokio_tungstenite::connect_async(request))
        .await
        .map_err(|_| failed(format!("no answer in {:?}", timing.connect_within)))?
        .map_err(|e| failed(e.to_string()))?;
    let (mut sink, mut stream) = socket.split();
    let (out, mut to_send) = mpsc::unbounded_channel::<String>();
    let (received, incoming) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        let mut ping = interval(timing.ping_every);
        let silence = sleep(timing.silent_for);
        tokio::pin!(silence);
        loop {
            tokio::select! {
                frame = to_send.recv() => match frame {
                    Some(text) => if sink.send(Message::Text(text)).await.is_err() { break },
                    None => break,
                },
                _ = ping.tick() => if sink.send(Message::Ping(Vec::new())).await.is_err() { break },
                frame = stream.next() => {
                    silence.as_mut().reset(Instant::now() + timing.silent_for);
                    match frame {
                        Some(Ok(Message::Text(text))) => if received.send(text).is_err() { break },
                        Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                        Some(Ok(_)) => {}
                    }
                }
                _ = &mut silence => {
                    tracing::info!("wsclient: no frame for {:?}, closing", timing.silent_for);
                    break;
                }
            }
        }
        let _ = sink.close().await;
    });
    Ok(Connection { out, incoming })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    const QUICK: Timing = Timing { ping_every: Duration::from_millis(50), silent_for: Duration::from_millis(300), connect_within: Duration::from_millis(300) };

    async fn listener() -> (TcpListener, String) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        (listener, url)
    }

    #[tokio::test]
    async fn frames_go_out_in_order_and_come_in() {
        let (listener, url) = listener().await;
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            while let Some(Ok(Message::Text(text))) = ws.next().await {
                ws.send(Message::Text(format!("echo {text}"))).await.unwrap();
            }
        });
        let mut conn = connect(url.as_str()).await.unwrap();
        for word in ["a", "b", "c"] {
            conn.out.send(word.into()).unwrap();
        }
        let mut got = Vec::new();
        for _ in 0..3 {
            got.push(conn.incoming.recv().await.unwrap());
        }
        assert_eq!(got, ["echo a", "echo b", "echo c"]);
    }

    #[tokio::test]
    async fn a_peer_that_answers_pings_is_not_silent() {
        let (listener, url) = listener().await;
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            while ws.next().await.is_some() {}
        });
        let mut conn = connect_with(url.as_str(), QUICK).await.unwrap();
        let waited = tokio::time::timeout(QUICK.silent_for * 3, conn.incoming.recv()).await;
        assert!(waited.is_err(), "still open after three times the silence limit");
    }

    #[tokio::test]
    async fn a_peer_gone_quiet_is_dropped() {
        let (listener, url) = listener().await;
        let (hold, held) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let _ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let _ = held.await;
        });
        let mut conn = connect_with(url.as_str(), QUICK).await.unwrap();
        let ended = tokio::time::timeout(QUICK.silent_for * 4, conn.incoming.recv()).await;
        assert_eq!(ended, Ok(None));
        drop(hold);
    }

    #[tokio::test]
    async fn an_unreachable_address_says_where() {
        let (listener, url) = listener().await;
        drop(listener);
        let Err(e) = connect(url.as_str()).await else { panic!("connected to nothing") };
        assert!(e.url.starts_with("ws://127.0.0.1"), "{e}");
    }
}
