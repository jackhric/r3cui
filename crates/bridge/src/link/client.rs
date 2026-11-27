use crate::link::{BridgeMsg, SampleFormat, SessionId, UiMsg, HOST, PORT};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{sleep, Instant};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use r3cui_sdk::ws::Backoff;

const EVENT_BACKLOG: usize = 256;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkEvent {
    Connected,
    Disconnected,
    Begin {
        session_id: SessionId,
        sample_rate: u32,
        channels: u16,
        format: SampleFormat,
    },
    /// PCM of the session of the last `Begin`.
    Audio {
        session_id: SessionId,
        pcm: Bytes,
    },
    End {
        session_id: SessionId,
    },
}

#[derive(Debug, Clone)]
pub struct LinkHandle {
    out: mpsc::UnboundedSender<UiMsg>,
}

impl LinkHandle {
    /// Sent if the link is up; a message sent while it is down is dropped (a session cannot outlive its link).
    pub fn send(&self, msg: UiMsg) {
        let _ = self.out.send(msg);
    }
}

/// The link to the bridge: events in, `UiMsg`s out. The task behind it ends when `events` is dropped.
#[derive(Debug)]
pub struct Link {
    pub events: mpsc::Receiver<LinkEvent>,
    pub handle: LinkHandle,
}

/// Connects to the bridge on `crate::link::HOST:PORT` and keeps reconnecting.
pub fn connect_loop() -> Link {
    connect_to(format!("{HOST}:{PORT}"))
}

/// As `connect_loop`, for a bridge at `addr` (`host:port`). Needs a running tokio runtime.
pub fn connect_to(addr: impl Into<String>) -> Link {
    let (event_tx, events) = mpsc::channel(EVENT_BACKLOG);
    let (out, out_rx) = mpsc::unbounded_channel();
    tokio::spawn(run(format!("ws://{}", addr.into()), event_tx, out_rx));
    Link {
        events,
        handle: LinkHandle { out },
    }
}

async fn run(
    url: String,
    events: mpsc::Sender<LinkEvent>,
    mut out: mpsc::UnboundedReceiver<UiMsg>,
) {
    let mut backoff = Backoff::new();
    loop {
        while out.try_recv().is_ok() {}
        backoff.begin_attempt();
        match tokio::time::timeout(HANDSHAKE_TIMEOUT, connect_async(&url)).await {
            Ok(Ok((ws, _))) => {
                let up = Instant::now();
                if !serve(ws, &events, &mut out).await {
                    return;
                }
                backoff.connection_ended(up.elapsed());
            }
            Ok(Err(e)) => tracing::debug!("bridge link {url}: {e}"),
            Err(_) => tracing::debug!("bridge link {url}: connect timed out"),
        }
        tokio::select! {
            _ = sleep(backoff.delay()) => {}
            _ = events.closed() => return,
        }
    }
}

/// Returns false when nobody is listening for events any more.
async fn serve<S>(
    mut ws: tokio_tungstenite::WebSocketStream<S>,
    events: &mpsc::Sender<LinkEvent>,
    out: &mut mpsc::UnboundedReceiver<UiMsg>,
) -> bool
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    if events.send(LinkEvent::Connected).await.is_err() {
        return false;
    }
    let mut session = None;
    let alive = loop {
        tokio::select! {
            frame = ws.next() => {
                let Some(Ok(frame)) = frame else { break true };
                let event = match frame {
                    Message::Text(text) => match serde_json::from_str::<BridgeMsg>(&text) {
                        Ok(BridgeMsg::Begin { session_id, sample_rate, channels, format }) => {
                            session = Some(session_id);
                            LinkEvent::Begin { session_id, sample_rate, channels, format }
                        }
                        Ok(BridgeMsg::End { session_id }) => {
                            session = None;
                            LinkEvent::End { session_id }
                        }
                        Err(e) => {
                            tracing::warn!("bridge link: unreadable frame {text:?}: {e}");
                            continue;
                        }
                    },
                    Message::Binary(pcm) => match session {
                        Some(session_id) => LinkEvent::Audio { session_id, pcm: pcm.into() },
                        None => {
                            tracing::warn!("bridge link: audio outside a session");
                            continue;
                        }
                    },
                    Message::Ping(payload) => {
                        if ws.send(Message::Pong(payload)).await.is_err() {
                            break true;
                        }
                        continue;
                    }
                    Message::Close(_) => break true,
                    _ => continue,
                };
                if events.send(event).await.is_err() {
                    break false;
                }
            }
            msg = out.recv() => {
                let Some(msg) = msg else { break false };
                let Ok(text) = serde_json::to_string(&msg) else { continue };
                if ws.send(Message::Text(text)).await.is_err() {
                    break true;
                }
            }
        }
    };
    alive && events.send(LinkEvent::Disconnected).await.is_ok()
}
