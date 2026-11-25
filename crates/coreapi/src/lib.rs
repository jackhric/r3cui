//! Minimal client for the UCR3 `remote-core` WebSocket API.
//!
//! Scaffold for milestone 1: connect, authenticate with the token file, subscribe to events,
//! issue requests and receive events. Message schemas are intentionally loose (`serde_json::Value`)
//! until we type them against `core-api`'s AsyncAPI spec. See `docs/core-ws-protocol.md`.

mod events;

pub use events::*;

use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

pub struct Config {
    pub url: String,
    pub token: String,
    /// `user:password` sent as HTTP Basic auth on the upgrade request. The core accepts the
    /// web-configurator PIN this way from the LAN, which is how a desktop build reaches a real remote.
    pub basic_auth: Option<String>,
}

impl Config {
    /// Read the launch env the firmware sets. On the desktop/simulator, `UC_TOKEN` can stand in for
    /// a token file path.
    pub fn from_env() -> Result<Self> {
        let url =
            std::env::var("UC_SOCKET_URL").unwrap_or_else(|_| "ws://127.0.0.1:8080/ws".to_string());
        let basic_auth = std::env::var("UC_BASIC_AUTH").ok();
        let token = match std::env::var("UC_TOKEN_PATH") {
            Ok(path) => std::fs::read_to_string(&path)
                .with_context(|| format!("reading token file {path}"))?
                .trim()
                .to_string(),
            Err(_) => match (std::env::var("UC_TOKEN"), &basic_auth) {
                (Ok(token), _) => token,
                (Err(_), Some(_)) => String::new(),
                (Err(_), None) => {
                    return Err(anyhow!("set UC_TOKEN_PATH (device), UC_TOKEN or UC_BASIC_AUTH (dev)"))
                }
            },
        };
        Ok(Self { url, token, basic_auth })
    }
}

#[derive(Debug, Clone)]
pub struct Event {
    pub msg: String,
    pub cat: Option<String>,
    pub data: Value,
}

#[derive(Debug)]
pub struct Response {
    pub msg: String,
    pub code: i64,
    pub data: Value,
}

impl Response {
    pub fn is_ok(&self) -> bool {
        (200..300).contains(&self.code)
    }

    pub fn ok(self) -> Result<Self> {
        if self.is_ok() {
            Ok(self)
        } else {
            let detail = self.data.get("message").and_then(Value::as_str).unwrap_or_default();
            Err(anyhow!("{} failed: code {} {detail}", self.msg, self.code))
        }
    }
}

struct Out {
    msg: String,
    data: Value,
    reply: oneshot::Sender<Response>,
}

/// Handle to a connected core session. Cloneable; requests are multiplexed over the one socket.
#[derive(Clone)]
pub struct Core {
    tx: mpsc::Sender<Out>,
}

impl Core {
    pub async fn request(&self, msg: &str, data: Value) -> Result<Response> {
        self.submit(msg, data).await?.await.map_err(|_| anyhow!("no response to {msg}"))
    }

    /// A request with no parameters, answered with its data when it succeeds.
    pub async fn fetch(&self, msg: &str) -> Option<Value> {
        self.request(msg, Value::Object(Default::default())).await.ok().filter(Response::is_ok).map(|r| r.data)
    }

    /// Queue a request without waiting for its answer. Requests reach the core in the order they
    /// are submitted, so a burst keeps its order while none of it waits on another's round trip.
    pub async fn submit(&self, msg: &str, data: Value) -> Result<oneshot::Receiver<Response>> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(Out { msg: msg.to_string(), data, reply }).await.map_err(|_| anyhow!("core task gone"))?;
        Ok(rx)
    }

    /// All configured entities, following the server's pagination to the end.
    pub async fn get_entities(&self) -> Result<Vec<Value>> {
        const LIMIT: usize = 100;
        let mut all = Vec::new();
        for page in 1.. {
            let resp = self
                .request("get_entities", json!({ "paging": { "page": page, "limit": LIMIT } }))
                .await?
                .ok()?;
            let items = match resp.data.get("entities") {
                Some(Value::Array(a)) => a.clone(),
                _ => return Err(anyhow!("get_entities: no `entities` array in {}", resp.msg)),
            };
            let total = resp.data.pointer("/paging/count").and_then(Value::as_u64).unwrap_or(0);
            let got = items.len();
            all.extend(items);
            if got < LIMIT || all.len() as u64 >= total {
                break;
            }
        }
        Ok(all)
    }

    pub async fn execute_entity_command(
        &self,
        entity_id: &str,
        cmd_id: &str,
        params: Value,
    ) -> Result<Response> {
        self.request(
            "execute_entity_command",
            json!({ "entity_id": entity_id, "cmd_id": cmd_id, "params": params }),
        )
        .await
    }
}

/// Connect, authenticate, subscribe to all events, and spawn the I/O task.
/// Returns the handle plus the stream of incoming events.
pub async fn connect(cfg: Config) -> Result<(Core, mpsc::Receiver<Event>)> {
    let mut request = cfg.url.as_str().into_client_request().with_context(|| format!("bad url {}", cfg.url))?;
    if let Some(basic) = &cfg.basic_auth {
        let value = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(basic));
        request.headers_mut().insert("Authorization", value.parse().context("auth header")?);
    }
    let (ws, _) = tokio_tungstenite::connect_async(request)
        .await
        .with_context(|| format!("ws connect {}", cfg.url))?;
    let (mut write, mut read) = ws.split();

    authenticate(&mut write, &mut read, &cfg.token).await?;
    tracing::info!("core authenticated at {}", cfg.url);

    let (out_tx, mut out_rx) = mpsc::channel::<Out>(32);
    let (ev_tx, ev_rx) = mpsc::channel::<Event>(256);

    tokio::spawn(async move {
        let mut next_id: u64 = 100;
        let mut pending: HashMap<u64, oneshot::Sender<Response>> = HashMap::new();

        // New sessions default to "all", but ask explicitly so behaviour is independent of that.
        let sub = json!({"kind":"req","id":next_id,"msg":"subscribe_events","msg_data":{}});
        next_id += 1;
        let _ = write.send(Message::Text(sub.to_string())).await;

        loop {
            tokio::select! {
                Some(out) = out_rx.recv() => {
                    let rid = next_id;
                    next_id += 1;
                    pending.insert(rid, out.reply);
                    let frame = json!({"kind":"req","id":rid,"msg":out.msg,"msg_data":out.data});
                    if write.send(Message::Text(frame.to_string())).await.is_err() {
                        break;
                    }
                }
                msg = read.next() => {
                    let Some(msg) = msg else { break };
                    let msg = match msg {
                        Ok(m) => m,
                        Err(e) => { tracing::warn!("ws read error: {e}"); break; }
                    };
                    match msg {
                        Message::Text(t) => {
                            if let Ok(v) = serde_json::from_str::<Value>(t.as_str()) {
                                dispatch(v, &mut pending, &ev_tx).await;
                            }
                        }
                        Message::Close(_) => break,
                        _ => {}
                    }
                }
                else => break,
            }
        }
        tracing::info!("core ws task ended");
    });

    Ok((Core { tx: out_tx }, ev_rx))
}

async fn dispatch(
    v: Value,
    pending: &mut HashMap<u64, oneshot::Sender<Response>>,
    ev_tx: &mpsc::Sender<Event>,
) {
    match v.get("kind").and_then(Value::as_str) {
        Some("resp") => {
            let resp = Response {
                msg: v.get("msg").and_then(Value::as_str).unwrap_or_default().to_string(),
                code: v.get("code").and_then(Value::as_i64).unwrap_or(0),
                data: v.get("msg_data").cloned().unwrap_or(Value::Null),
            };
            if let Some(id) = v.get("req_id").and_then(Value::as_u64) {
                if let Some(tx) = pending.remove(&id) {
                    let _ = tx.send(resp);
                }
            }
        }
        Some("event") => {
            let ev = Event {
                msg: v.get("msg").and_then(Value::as_str).unwrap_or_default().to_string(),
                cat: v.get("cat").and_then(Value::as_str).map(String::from),
                data: v.get("msg_data").cloned().unwrap_or(Value::Null),
            };
            let _ = ev_tx.send(ev).await;
        }
        _ => {}
    }
}

type Writer = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;
type Reader = futures_util::stream::SplitStream<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
>;

/// Message-based handshake: wait for `auth_required`, reply with the token, expect `authentication`
/// code 200. If the server never sends `auth_required` (header-auth path), assume it accepted us.
async fn authenticate(write: &mut Writer, read: &mut Reader, token: &str) -> Result<()> {
    loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(3)) => return Ok(()),
            msg = read.next() => {
                let Some(msg) = msg else { return Err(anyhow!("connection closed during auth")) };
                if let Message::Text(t) = msg? {
                    let v: Value = serde_json::from_str(t.as_str()).unwrap_or(Value::Null);
                    match v.get("msg").and_then(Value::as_str) {
                        Some("auth_required") => {
                            let frame = json!({"kind":"req","id":1,"msg":"auth","msg_data":{"token":token}});
                            write.send(Message::Text(frame.to_string())).await
                                .map_err(|e| anyhow!("send auth: {e}"))?;
                        }
                        Some("authentication") => {
                            let code = v.get("code").and_then(Value::as_i64).unwrap_or(0);
                            return if code == 200 { Ok(()) }
                                   else { Err(anyhow!("authentication failed: code {code}")) };
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}
