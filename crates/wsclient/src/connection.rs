use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{interval, sleep, Instant};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::Connector;

use crate::tls::{self, Trust};

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

/// What goes out on a connection: text or binary frames, in one queue so they keep the order given.
#[derive(Clone)]
pub struct Outbox(mpsc::UnboundedSender<Message>);

/// The connection is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("connection closed")]
pub struct Closed;

impl Outbox {
    pub fn send(&self, text: String) -> Result<(), Closed> {
        self.0.send(Message::Text(text)).map_err(|_| Closed)
    }

    pub fn send_binary(&self, bytes: Vec<u8>) -> Result<(), Closed> {
        self.0.send(Message::Binary(bytes)).map_err(|_| Closed)
    }
}

/// One open connection: frames to send, in the order given, and text frames received (binary frames from the peer
/// are dropped: no client here has a use for them). `incoming` ends when the socket does, however it ended;
/// dropping `incoming`, or every clone of `out`, closes the socket.
pub struct Connection {
    pub out: Outbox,
    pub incoming: mpsc::UnboundedReceiver<String>,
}

pub async fn connect(request: impl IntoClientRequest) -> Result<Connection, ConnectError> {
    connect_with(request, Timing::default()).await
}

pub async fn connect_trusting(request: impl IntoClientRequest, trust: Trust) -> Result<Connection, ConnectError> {
    open(request, Timing::default(), trust).await
}

pub async fn connect_with(request: impl IntoClientRequest, timing: Timing) -> Result<Connection, ConnectError> {
    open(request, timing, Trust::Verified).await
}

async fn open(request: impl IntoClientRequest, timing: Timing, trust: Trust) -> Result<Connection, ConnectError> {
    let request = request.into_client_request().map_err(|e| ConnectError { url: String::new(), reason: e.to_string() })?;
    let url = request.uri().to_string();
    let failed = |reason: String| ConnectError { url: url.clone(), reason };
    let connector = Connector::Rustls(tls::client_config(trust).map_err(|e| failed(e.to_string()))?);
    let connecting = tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(connector));
    let (socket, _) = tokio::time::timeout(timing.connect_within, connecting)
        .await
        .map_err(|_| failed(format!("no answer in {:?}", timing.connect_within)))?
        .map_err(|e| failed(e.to_string()))?;
    let (mut sink, mut stream) = socket.split();
    let (out, mut to_send) = mpsc::unbounded_channel::<Message>();
    let (received, incoming) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        let mut ping = interval(timing.ping_every);
        let silence = sleep(timing.silent_for);
        tokio::pin!(silence);
        loop {
            tokio::select! {
                frame = to_send.recv() => match frame {
                    Some(frame) => if sink.send(frame).await.is_err() { break },
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
    Ok(Connection { out: Outbox(out), incoming })
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
    async fn binary_frames_keep_their_place_among_text() {
        let (listener, url) = listener().await;
        let (seen, mut seen_rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            while let Some(Ok(frame)) = ws.next().await {
                if frame.is_text() || frame.is_binary() {
                    seen.send(frame).unwrap();
                }
            }
        });
        let conn = connect(url.as_str()).await.unwrap();
        conn.out.send("a".into()).unwrap();
        conn.out.send_binary(vec![1, 2, 3]).unwrap();
        conn.out.send("b".into()).unwrap();
        let mut got = Vec::new();
        for _ in 0..3 {
            got.push(seen_rx.recv().await.unwrap());
        }
        assert_eq!(got, [Message::Text("a".into()), Message::Binary(vec![1, 2, 3]), Message::Text("b".into())]);
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
