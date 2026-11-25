//! The running client: connects (and reconnects, backing off), authenticates, subscribes to the watched entities,
//! runs the engine over what arrives and publishes `HaState`.

use crate::client::Client;
use crate::config::HaConfig;
use crate::engine::{Engine, Event};
use crate::error::{Error, Result};
use crate::handle::HaHandle;
use crate::ids::EntityId;
use crate::state::HaState;
use crate::transport::{self, Connection};
use crate::wire::{Compressed, Incoming};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;

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

async fn run(cfg: HaConfig, state: watch::Sender<HaState>, slot: Arc<Mutex<Option<Client>>>, mut watched: watch::Receiver<Vec<EntityId>>) {
    let mut engine = Engine::default();
    let url = match cfg.ws_url() {
        Ok(url) => Ok(url),
        Err(Error::NotConfigured) => return,
        Err(e) => Err(e),
    };
    let mut attempt = 0u32;
    loop {
        if watched.has_changed().is_err() {
            return;
        }
        attempt += 1;
        engine.apply(Event::Connecting);
        state.send_replace(engine.state.clone());
        let error = match &url {
            Ok(url) => match transport::connect(url).await {
                Ok(conn) => {
                    let up = Instant::now();
                    let error = connected(conn, &cfg.token, &mut engine, &state, &slot, &mut watched).await;
                    *slot.lock().unwrap() = None;
                    // A connection that stayed up a while starts the backoff over.
                    if up.elapsed() > Duration::from_secs(30) {
                        attempt = 1;
                    }
                    error
                }
                Err(e) => e,
            },
            Err(e) => e.clone(),
        };
        let retry_in = match &error {
            Error::Rejected(_) | Error::Unsupported(_) => REJECTED_RETRY,
            _ => backoff(attempt),
        };
        tracing::info!("homeassistant: {error}; again in {retry_in:?}");
        engine.apply(match error {
            Error::Rejected(message) => Event::Rejected { message },
            error => Event::Disconnected { error, retry_in },
        });
        state.send_replace(engine.state.clone());
        tokio::time::sleep(retry_in).await;
    }
}

fn backoff(attempt: u32) -> Duration {
    const STEPS: [u64; 5] = [500, 1000, 2000, 4000, 8000];
    Duration::from_millis(STEPS.get(attempt.saturating_sub(1) as usize).copied().unwrap_or(15_000))
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
    conn.out.send(json!({ "type": "auth", "access_token": token }).to_string()).map_err(|_| Error::Closed)?;
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
    let (id, reply) = client.request("subscribe_entities", json!({ "entity_ids": ids }));
    tokio::spawn(async move {
        if let Err(e) = reply.wait(READ).await {
            tracing::info!("homeassistant: subscribe_entities: {e}");
        }
    });
    Some(id)
}

/// One connection's life; returns why it ended.
async fn connected(
    mut conn: Connection,
    token: &str,
    engine: &mut Engine,
    state: &watch::Sender<HaState>,
    slot: &Arc<Mutex<Option<Client>>>,
    watched: &mut watch::Receiver<Vec<EntityId>>,
) -> Error {
    let version = match authenticate(&mut conn, token).await {
        Ok(version) => version,
        Err(e) => return e,
    };
    tracing::info!("homeassistant: connected to {}", version.as_deref().unwrap_or("Home Assistant"));
    let Connection { out, mut incoming } = conn;
    let client = Client::new(out);
    *slot.lock().unwrap() = Some(client.clone());
    engine.apply(Event::Authenticated { version });
    let mut sub = subscribe(&client, &watched.borrow_and_update().clone());
    state.send_replace(engine.state.clone());
    loop {
        tokio::select! {
            frame = incoming.recv() => {
                let Some(frame) = frame else { return Error::Closed };
                match Incoming::parse(&frame) {
                    Ok(Incoming::Reply { id, outcome }) => client.deliver(id, outcome),
                    Ok(Incoming::Event { id, event }) if Some(id) == sub => match serde_json::from_value::<Compressed>(event) {
                        Ok(c) => {
                            engine.apply(Event::Entities(c));
                            state.send_replace(engine.state.clone());
                        }
                        Err(e) => tracing::debug!("homeassistant event: {e}"),
                    },
                    Ok(_) => {}
                    Err(e) => tracing::debug!("homeassistant frame: {e}"),
                }
            }
            changed = watched.changed() => {
                if changed.is_err() {
                    return Error::Closed;
                }
                if let Some(old) = sub.take() {
                    let _ = client.request("unsubscribe_events", json!({ "subscription": old }));
                }
                engine.apply(Event::Resubscribing);
                sub = subscribe(&client, &watched.borrow_and_update().clone());
                state.send_replace(engine.state.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_steps_then_settles() {
        let secs = |a| backoff(a).as_secs_f64();
        assert_eq!([secs(1), secs(2), secs(3), secs(4), secs(5), secs(6), secs(60)], [0.5, 1.0, 2.0, 4.0, 8.0, 15.0, 15.0]);
    }
}
