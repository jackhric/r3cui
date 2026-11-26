//! The running client: connects (and reconnects, backing off), authenticates, subscribes to the watched entities,
//! runs the engine over what arrives and publishes `HaState`.

use crate::client::Client;
use crate::config::HaConfig;
use crate::engine::{Engine, Event};
use crate::error::{Error, Result};
use crate::handle::HaHandle;
use crate::ids::EntityId;
use crate::protocol::{field, MessageType};
use crate::state::HaState;
use crate::wire::{Compressed, Incoming};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;
use wsclient::{Connection, Session};

const READ: Duration = Duration::from_secs(5);
const AUTH: Duration = Duration::from_secs(5);
// A refused token stays refused until someone changes it.
const REJECTED_RETRY: Duration = Duration::from_secs(60);

pub struct HaService;

impl HaService {
    /// Start following Home Assistant on the current tokio runtime.
    pub fn start(cfg: HaConfig, watched: Vec<EntityId>) -> HaHandle {
        let (state_tx, state_rx) = watch::channel(HaState::default());
        let (watch_tx, watch_rx) = watch::channel(watched);
        let slot: Arc<Mutex<Option<Client>>> = Arc::default();
        tokio::spawn(run(cfg.clone(), state_tx, slot.clone(), watch_rx));
        HaHandle::new(cfg, state_rx, slot, watch_tx)
    }
}

async fn run(cfg: HaConfig, state: watch::Sender<HaState>, slot: Arc<Mutex<Option<Client>>>, watched: watch::Receiver<Vec<EntityId>>) {
    let url = match cfg.ws_url() {
        Ok(url) => Ok(url),
        Err(Error::NotConfigured) => return,
        Err(e) => Err(e),
    };
    wsclient::reconnect(&mut Following { token: cfg.token, url, engine: Engine::default(), state, slot, watched }).await;
}

struct Following {
    token: String,
    url: Result<String>,
    engine: Engine,
    state: watch::Sender<HaState>,
    slot: Arc<Mutex<Option<Client>>>,
    watched: watch::Receiver<Vec<EntityId>>,
}

impl Following {
    fn publish(&self) {
        self.state.send_replace(self.engine.state.clone());
    }

    /// One connection's life; returns why it ended.
    async fn follow(&mut self, mut conn: Connection) -> Error {
        let version = match authenticate(&mut conn, &self.token).await {
            Ok(version) => version,
            Err(e) => return e,
        };
        tracing::info!("homeassistant: connected to {}", version.as_deref().unwrap_or("Home Assistant"));
        let Connection { out, mut incoming } = conn;
        let client = Client::new(out);
        *self.slot.lock().unwrap() = Some(client.clone());
        self.engine.apply(Event::Authenticated { version });
        let mut sub = subscribe(&client, &self.watched.borrow_and_update().clone());
        self.publish();
        loop {
            tokio::select! {
                frame = incoming.recv() => {
                    let Some(frame) = frame else { return Error::Closed };
                    match Incoming::parse(&frame) {
                        Ok(Incoming::Reply { id, outcome }) => client.deliver(id, outcome),
                        Ok(Incoming::Event { id, event }) if Some(id) == sub => match serde_json::from_value::<Compressed>(event) {
                            Ok(c) => {
                                self.engine.apply(Event::Entities(c));
                                self.publish();
                            }
                            Err(e) => tracing::debug!("homeassistant event: {e}"),
                        },
                        Ok(_) => {}
                        Err(e) => tracing::debug!("homeassistant frame: {e}"),
                    }
                }
                changed = self.watched.changed() => {
                    if changed.is_err() {
                        return Error::Closed;
                    }
                    if let Some(old) = sub.take() {
                        let _ = client.request(MessageType::UnsubscribeEvents, json!({ field::SUBSCRIPTION: old }));
                    }
                    self.engine.apply(Event::Resubscribing);
                    sub = subscribe(&client, &self.watched.borrow_and_update().clone());
                    self.publish();
                }
            }
        }
    }
}

impl Session for Following {
    type Error = Error;

    fn connecting(&mut self, _attempt: u32) -> bool {
        if self.watched.has_changed().is_err() {
            return false;
        }
        self.engine.apply(Event::Connecting);
        self.publish();
        true
    }

    async fn connect(&mut self) -> Result<Connection> {
        match &self.url {
            Ok(url) => Ok(wsclient::connect(url.as_str()).await?),
            Err(e) => Err(e.clone()),
        }
    }

    async fn serve(&mut self, conn: Connection) -> Error {
        let error = self.follow(conn).await;
        *self.slot.lock().unwrap() = None;
        error
    }

    fn retry_in(&self, error: &Error, backoff: Duration) -> Duration {
        match error {
            Error::Rejected(_) | Error::Unsupported(_) => REJECTED_RETRY,
            _ => backoff,
        }
    }

    fn lost(&mut self, error: Error, retry_in: Duration) {
        tracing::info!("homeassistant: {error}; again in {retry_in:?}");
        self.engine.apply(match error {
            Error::Rejected(message) => Event::Rejected { message },
            error => Event::Disconnected { error, retry_in },
        });
        self.publish();
    }
}

/// The handshake: Home Assistant speaks first, we answer with the token, it says whether that was good.
async fn authenticate(conn: &mut Connection, token: &str) -> Result<Option<String>> {
    let next = async |conn: &mut Connection| -> Result<Incoming> {
        let frame = tokio::time::timeout(AUTH, conn.incoming.recv()).await.map_err(|_| Error::Timeout)?.ok_or(Error::Closed)?;
        Incoming::parse(&frame)
    };
    match next(conn).await? {
        Incoming::AuthRequired => {}
        other => return Err(Error::Decode(format!("expected auth_required, got {other:?}"))),
    }
    let auth = json!({ field::TYPE: MessageType::Auth.as_str(), field::ACCESS_TOKEN: token });
    conn.out.send(auth.to_string()).map_err(|_| Error::Closed)?;
    match next(conn).await? {
        Incoming::AuthOk { version } => Ok(version),
        Incoming::AuthInvalid { message } => Err(Error::Rejected(message)),
        other => Err(Error::Decode(format!("expected auth_ok, got {other:?}"))),
    }
}

/// Subscribes to `ids`; the subscription's id is that of the request, and its events carry it.
fn subscribe(client: &Client, ids: &[EntityId]) -> Option<u32> {
    if ids.is_empty() {
        return None;
    }
    let (id, reply) = client.request(MessageType::SubscribeEntities, json!({ field::ENTITY_IDS: ids }));
    tokio::spawn(async move {
        if let Err(e) = reply.wait(READ).await {
            tracing::info!("homeassistant: {}: {e}", MessageType::SubscribeEntities.as_str());
        }
    });
    Some(id)
}
