//! Client for the UCR3 `remote-core` WebSocket API, for the remote's own state: connect, authenticate with the
//! token file, receive its events (typed in `events`) and make requests. Other programs are r3cui's drivers, not the
//! core's integrations, so entities and their commands aren't here. See `docs/core-ws-protocol.md`.

mod events;
pub mod messages;
mod system;
mod voice;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use events::*;
pub use system::*;
pub use voice::*;

use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use wsclient::{Connection, Pending};

const SOCKET_URL_ENV: &str = "UC_SOCKET_URL";
const DEFAULT_SOCKET_URL: &str = "ws://127.0.0.1:8080/ws";
const AUTH_REQUEST_ID: u64 = 1;
const SUBSCRIBE_REQUEST_ID: u64 = 100;
const FIRST_REQUEST_ID: u64 = 101;
// A core that never sends `auth_required` accepted the header auth.
const AUTH_PROMPT_WAIT: Duration = Duration::from_secs(3);
const EVENT_BACKLOG: usize = 256;

pub struct Config {
    pub url: String,
    pub token: String,
    /// `user:password` sent as HTTP Basic auth on the upgrade request. The core accepts the
    /// web-configurator PIN this way from the LAN, which is how a desktop build reaches a real remote.
    pub basic_auth: Option<String>,
}

impl Config {
    /// The core's WebSocket address, from `UC_SOCKET_URL`.
    pub fn url_from_env() -> String {
        std::env::var(SOCKET_URL_ENV).unwrap_or_else(|_| DEFAULT_SOCKET_URL.to_string())
    }

    /// Read the launch env the firmware sets. On the desktop/simulator, `UC_TOKEN` can stand in for
    /// a token file path.
    pub fn from_env() -> Result<Self> {
        let url = Self::url_from_env();
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

/// Handle to a connected core session. Cloneable; requests are multiplexed over the one socket, which closes when
/// the last handle is dropped.
#[derive(Clone)]
pub struct Core {
    out: mpsc::UnboundedSender<String>,
    pending: Pending<u64, Response>,
    next_id: Arc<AtomicU64>,
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
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let reply = self.pending.insert(id);
        if self.out.send(request_frame(id, msg, data)).is_err() {
            self.pending.cancel(&id);
            return Err(anyhow!("core task gone"));
        }
        Ok(reply)
    }
}

fn request_frame(id: u64, msg: &str, data: Value) -> String {
    json!({"kind":"req","id":id,"msg":msg,"msg_data":data}).to_string()
}

/// Connect, authenticate, subscribe to all events, and spawn the I/O task.
/// Returns the handle plus the stream of incoming events.
pub async fn connect(cfg: Config) -> Result<(Core, mpsc::Receiver<Event>)> {
    let mut request = cfg.url.as_str().into_client_request().with_context(|| format!("bad url {}", cfg.url))?;
    if let Some(basic) = &cfg.basic_auth {
        let value = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(basic));
        request.headers_mut().insert("Authorization", value.parse().context("auth header")?);
    }
    let mut conn = wsclient::connect(request).await.map_err(|e| anyhow!("ws connect {}: {}", cfg.url, e.reason))?;

    authenticate(&mut conn, &cfg.token).await?;
    tracing::info!("core authenticated at {}", cfg.url);

    let Connection { out, mut incoming } = conn;
    let (ev_tx, ev_rx) = mpsc::channel::<Event>(EVENT_BACKLOG);
    let pending = Pending::new();

    // New sessions default to "all", but ask explicitly so behaviour is independent of that.
    let _ = out.send(request_frame(SUBSCRIBE_REQUEST_ID, messages::SUBSCRIBE_EVENTS, json!({})));

    tokio::spawn({
        let pending = pending.clone();
        async move {
            while let Some(text) = incoming.recv().await {
                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                    dispatch(v, &pending, &ev_tx).await;
                }
            }
            pending.clear();
            tracing::info!("core ws task ended");
        }
    });

    Ok((Core { out, pending, next_id: Arc::new(AtomicU64::new(FIRST_REQUEST_ID)) }, ev_rx))
}

async fn dispatch(
    v: Value,
    pending: &Pending<u64, Response>,
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
                pending.deliver(&id, resp);
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

/// Message-based handshake: wait for `auth_required`, reply with the token, expect `authentication`
/// code 200. If the server never sends `auth_required` (header-auth path), assume it accepted us.
async fn authenticate(conn: &mut Connection, token: &str) -> Result<()> {
    loop {
        let Ok(frame) = tokio::time::timeout(AUTH_PROMPT_WAIT, conn.incoming.recv()).await else { return Ok(()) };
        let Some(text) = frame else { return Err(anyhow!("connection closed during auth")) };
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        match v.get("msg").and_then(Value::as_str) {
            Some(messages::AUTH_REQUIRED) => {
                let frame = request_frame(AUTH_REQUEST_ID, messages::AUTH, json!({"token":token}));
                conn.out.send(frame).map_err(|_| anyhow!("send auth: connection closed"))?;
            }
            Some(messages::AUTHENTICATION) => {
                let code = v.get("code").and_then(Value::as_i64).unwrap_or(0);
                return if code == 200 { Ok(()) } else { Err(anyhow!("authentication failed: code {code}")) };
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeCore, Shared};
    use wsclient::testing::FakeServer;

    async fn fake_core() -> (FakeServer<Value>, Shared) {
        FakeCore::start("t").await
    }

    fn config(server: &FakeServer<Value>, token: &str) -> Config {
        Config { url: format!("ws://{}", server.addr()), token: token.into(), basic_auth: None }
    }

    #[tokio::test]
    async fn requests_are_answered_and_events_arrive() {
        let (server, _) = fake_core().await;
        let (core, mut events) = connect(config(&server, "t")).await.unwrap();
        let version = core.fetch(messages::VERSION).await.unwrap();
        assert_eq!(version["os"], "2.8.4");
        let event = events.recv().await.unwrap();
        assert_eq!((event.msg.as_str(), event.data["capacity"].as_i64()), ("battery_status", Some(80)));
        let refused = core.request("unknown", json!({})).await.unwrap();
        assert_eq!(refused.code, 404);
        assert!(refused.ok().is_err());
    }

    #[tokio::test]
    async fn a_wrong_token_fails_the_connection() {
        let (server, _) = fake_core().await;
        let Err(e) = connect(config(&server, "wrong")).await else { panic!("connected with a bad token") };
        assert!(e.to_string().contains("401"), "{e}");
    }

    #[tokio::test]
    async fn a_dropped_connection_ends_the_events_and_fails_requests() {
        let (server, _) = fake_core().await;
        let (core, mut events) = connect(config(&server, "t")).await.unwrap();
        server.kick();
        assert!(events.recv().await.is_none());
        assert!(core.request(messages::VERSION, json!({})).await.is_err());
    }

    #[tokio::test]
    async fn version_and_ui_info_round_trip() {
        let (server, state) = fake_core().await;
        let (core, _events) = connect(config(&server, "t")).await.unwrap();
        let info = core.call(&GetVersion {}).await.unwrap();
        assert_eq!((info.model.as_str(), info.address.as_str(), info.os.as_str()), ("UCR3", "192.168.1.40", "2.8.4"));
        core.announce_ui("0.1.0").await.unwrap();
        assert_eq!(state.lock().unwrap().ui_info, Some(json!({"version": "0.1.0", "entity_types": []})));
    }

    #[tokio::test]
    async fn a_firmware_update_is_found_downloaded_then_installed() {
        let (server, state) = fake_core().await;
        let (core, mut events) = connect(config(&server, "t")).await.unwrap();
        let info = core.call(&CheckSystemUpdate { force_update: true }).await.unwrap();
        assert_eq!(state.lock().unwrap().forced, Some(true));
        assert_eq!((info.installed_version.as_str(), info.available.len()), ("2.8.4", 1));
        let update = &info.available[0];
        assert_eq!((update.version.as_str(), update.download, update.size), ("2.9.0", DownloadState::Pending, 120_000_000));
        assert_eq!(localized(&update.description, "en"), Some("Faster Wi-Fi."));

        assert_eq!(core.update_system(&update.id).await, Ok(UpdateStart::Download));
        let event = events.recv().await.unwrap();
        let CoreEvent::SoftwareUpdate(started) = event.typed() else { panic!("{event:?}") };
        assert_eq!((started.event_type, started.update_id.as_str()), (UpdateEventType::Start, "u-2.9.0"));
        let info = core.call(&CheckSystemUpdate { force_update: false }).await.unwrap();
        assert_eq!(info.available[0].download, DownloadState::Downloaded);
        assert_eq!(core.update_system(&update.id).await, Ok(UpdateStart::Start));
    }

    #[tokio::test]
    async fn a_low_battery_refuses_the_update() {
        let (server, state) = fake_core().await;
        state.lock().unwrap().battery_ok = false;
        let (core, _events) = connect(config(&server, "t")).await.unwrap();
        assert_eq!(core.update_system("u-2.9.0").await, Err(UpdateRefusal::BatteryLow));
    }

    #[tokio::test]
    async fn web_configurator_access_is_switched_with_a_pin() {
        let (server, state) = fake_core().await;
        let (core, _events) = connect(config(&server, "t")).await.unwrap();
        assert!(!core.api_access().await.unwrap().web_configurator.enabled);
        core.enable_web_configurator("4827").await.unwrap();
        assert!(core.api_access().await.unwrap().web_configurator.enabled);
        assert_eq!(state.lock().unwrap().web_pin.as_deref(), Some("4827"));
        core.disable_web_configurator().await.unwrap();
        assert!(!core.api_access().await.unwrap().web_configurator.enabled);
    }

    #[tokio::test]
    async fn a_voice_session_round_trips() {
        let (server, state) = fake_core().await;
        let (core, mut events) = connect(config(&server, "t")).await.unwrap();

        let cfg = core.get_voice_control_cfg().await.unwrap();
        assert!(cfg.microphone);
        let assistants = core.get_voice_assistants().await.unwrap();
        assert_eq!((assistants.len(), assistants[0].entity_id.0.as_str()), (1, "r3cui.voice"));

        let off = VoiceControlUpdate { microphone: Some(false), voice_assistant: None };
        assert!(!core.set_voice_control_cfg(&off).await.unwrap().microphone);

        let entity = assistants[0].entity_id.clone();
        core.voice_start(&entity, &VoiceStart::new(7)).await.unwrap();
        server.broadcast(json!({"type": "ready", "entity_id": "r3cui.voice", "session_id": 7}));
        let CoreEvent::Assistant(ready) = events.recv().await.unwrap().typed() else { panic!("not an assistant event") };
        assert_eq!((ready.session_id, ready.kind), (7, AssistantEventKind::Ready));

        core.voice_end(&entity, None).await.unwrap();
        let sent = state.lock().unwrap().entity_commands.clone();
        let commands: Vec<_> = sent.iter().map(|c| (c.entity_id.as_str(), c.cmd_id.as_str())).collect();
        assert_eq!(commands, [("r3cui.voice", messages::VOICE_START), ("r3cui.voice", messages::VOICE_END)]);
        assert_eq!(sent[0].params, Some(json!({"session_id": 7, "timeout": 15, "speech_response": false})));
        assert_eq!(sent[1].params, None);
    }
}
