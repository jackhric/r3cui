//! The listener: health over plain HTTP, everything else over one WebSocket after the key is proved.

use super::act;
use crate::app::UiAction;
use crate::AppWindow;
use futures_util::{SinkExt, StreamExt};
use remotewire::{Challenge, Envelope, Hello, Reply, Request};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;

/// The QR code on the settings page, in pixels a side at most.
pub(crate) const QR_SIZE: u32 = 176;
const HELLO_WAIT: Duration = Duration::from_secs(10);
const HEALTH: &[u8] = b"GET /health";
/// After this many wrong proofs, nobody gets in for a while.
const MISSES_ALLOWED: u32 = 5;
const LOCKOUT: Duration = Duration::from_secs(60);

static MISSES: AtomicU32 = AtomicU32::new(0);
static LOCKED_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);

/// This remote's address on the network as `ip:port`.
pub(super) fn address() -> String {
    format!("{}:{}", lan_ip(), remotewire::PORT)
}

/// This remote's address on the network: the route out picks it, without sending anything.
pub(crate) fn lan_ip() -> String {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| s.connect("192.0.2.1:9").and_then(|()| s.local_addr()))
        .map_or_else(|_| "127.0.0.1".to_string(), |a| a.ip().to_string())
}

/// Listens while there is a key, stops while there is none.
pub(super) async fn run(mut listen: watch::Receiver<Option<String>>, weak: slint::Weak<AppWindow>, actions: mpsc::UnboundedSender<UiAction>) {
    loop {
        let key = listen.borrow_and_update().clone();
        let Some(key) = key else {
            if listen.changed().await.is_err() {
                return;
            }
            continue;
        };
        let addr = SocketAddr::from(([0, 0, 0, 0], remotewire::PORT));
        let listener = match TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                tracing::error!("remote control: cannot listen on {addr}: {e}");
                if listen.changed().await.is_err() {
                    return;
                }
                continue;
            }
        };
        tracing::info!("remote control: listening on {addr}");
        loop {
            tokio::select! {
                changed = listen.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    tracing::info!("remote control: stopped listening");
                    break;
                }
                accepted = listener.accept() => {
                    let Ok((stream, peer)) = accepted else { continue };
                    if !local(peer.ip()) {
                        tracing::warn!("remote control: refused {peer}: not on the local network");
                        continue;
                    }
                    let (key, weak, actions) = (key.clone(), weak.clone(), actions.clone());
                    tokio::spawn(async move {
                        if let Err(e) = connection(stream, &key, weak, actions).await {
                            tracing::info!("remote control: {peer}: {e}");
                        }
                    });
                }
            }
        }
    }
}

/// Loopback, link-local or a private range: never an address from the internet.
pub(crate) fn local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unicast_link_local() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_private() || v4.is_loopback()),
    }
}

fn locked() -> bool {
    LOCKED_UNTIL.lock().unwrap().is_some_and(|until| Instant::now() < until)
}

fn missed() {
    if MISSES.fetch_add(1, Ordering::SeqCst) + 1 >= MISSES_ALLOWED {
        MISSES.store(0, Ordering::SeqCst);
        *LOCKED_UNTIL.lock().unwrap() = Some(Instant::now() + LOCKOUT);
        tracing::warn!("remote control: too many wrong keys, locked for {}s", LOCKOUT.as_secs());
    }
}

async fn connection(mut stream: TcpStream, key: &str, weak: slint::Weak<AppWindow>, actions: mpsc::UnboundedSender<UiAction>) -> anyhow::Result<()> {
    let mut first = [0u8; HEALTH.len()];
    let n = stream.peek(&mut first).await?;
    if first[..n] == *HEALTH {
        let body = b"r3cui\n";
        let head = format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
        stream.write_all(head.as_bytes()).await?;
        stream.write_all(body).await?;
        return Ok(());
    }
    let mut ws = tokio_tungstenite::accept_async(stream).await?;
    if locked() {
        anyhow::bail!("locked after wrong keys");
    }
    let mut nonce = [0u8; 16];
    getrandom::getrandom(&mut nonce)?;
    let nonce: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
    ws.send(Message::text(serde_json::to_string(&Challenge::Challenge { nonce: nonce.clone() })?)).await?;
    let hello = tokio::time::timeout(HELLO_WAIT, ws.next()).await?.ok_or_else(|| anyhow::anyhow!("gone before hello"))??;
    let Hello::Hello { proof } = serde_json::from_str(hello.to_text()?)?;
    if !remotewire::same(&proof, &remotewire::proof(key, &nonce)) {
        missed();
        anyhow::bail!("wrong key");
    }
    MISSES.store(0, Ordering::SeqCst);
    tracing::info!("remote control: a client is in");
    act::attached(&weak, 1);
    let result = requests(&mut ws, &weak, &actions).await;
    act::attached(&weak, -1);
    result
}

async fn requests(ws: &mut tokio_tungstenite::WebSocketStream<TcpStream>, weak: &slint::Weak<AppWindow>, actions: &mpsc::UnboundedSender<UiAction>) -> anyhow::Result<()> {
    while let Some(message) = ws.next().await {
        let message = message?;
        let Message::Text(text) = message else {
            if message.is_close() {
                break;
            }
            continue;
        };
        let request: Envelope<Request> = match serde_json::from_str(&text) {
            Ok(r) => r,
            Err(e) => {
                let reply = Envelope { id: 0, body: Reply::Error { message: format!("not a request: {e}") } };
                ws.send(Message::text(serde_json::to_string(&reply)?)).await?;
                continue;
            }
        };
        let (reply, png) = act::run(request.body, weak, actions).await;
        ws.send(Message::text(serde_json::to_string(&Envelope { id: request.id, body: reply })?)).await?;
        if let Some(png) = png {
            ws.send(Message::binary(png)).await?;
        }
    }
    Ok(())
}
