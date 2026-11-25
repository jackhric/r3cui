//! `FakeHa`: an in-process Home Assistant WebSocket server for tests. It speaks the real protocol (auth, ping,
//! `get_states`, `subscribe_entities` with compressed diffs, `call_service`) and gives media players the effects
//! the services have in Home Assistant.

use crate::config::HaConfig;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;

#[derive(Clone, Debug, PartialEq)]
struct Entity {
    state: String,
    attributes: Map<String, Value>,
}

#[derive(Clone, Debug)]
enum Push {
    Set { id: String, old: Option<Entity>, new: Entity },
    Kick,
}

struct Shared {
    token: String,
    entities: Mutex<BTreeMap<String, Entity>>,
    calls: Mutex<Vec<(String, String, Value)>>,
    connections: AtomicUsize,
    pushes: broadcast::Sender<Push>,
}

pub struct FakeHa {
    addr: SocketAddr,
    shared: Arc<Shared>,
}

impl FakeHa {
    pub async fn start(token: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let shared = Arc::new(Shared {
            token: token.to_owned(),
            entities: Mutex::default(),
            calls: Mutex::default(),
            connections: AtomicUsize::new(0),
            pushes: broadcast::channel(256).0,
        });
        let accepting = shared.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(serve(stream, accepting.clone()));
            }
        });
        Self { addr, shared }
    }

    pub fn config(&self) -> HaConfig {
        HaConfig::new(format!("http://{}", self.addr), self.shared.token.clone())
    }

    /// Sets an entity (creating it if need be) and tells whoever subscribed. `attributes` is a JSON object.
    pub fn set_entity(&self, id: &str, state: &str, attributes: Value) {
        let attributes = attributes.as_object().cloned().unwrap_or_default();
        self.shared.update(id, true, |e| *e = Entity { state: state.to_owned(), attributes });
    }

    /// Service calls received so far: domain, service and `service_data`.
    pub fn calls(&self) -> Vec<(String, String, Value)> {
        self.shared.calls.lock().unwrap().clone()
    }

    /// Drops every connection.
    pub fn kick(&self) {
        let _ = self.shared.pushes.send(Push::Kick);
    }

    /// Connections accepted so far.
    pub fn connections(&self) -> usize {
        self.shared.connections.load(Ordering::SeqCst)
    }
}

impl Shared {
    fn update(&self, id: &str, create: bool, change: impl FnOnce(&mut Entity)) {
        let mut entities = self.entities.lock().unwrap();
        let old = entities.get(id).cloned();
        if old.is_none() && !create {
            return;
        }
        let mut new = old.clone().unwrap_or(Entity { state: String::new(), attributes: Map::new() });
        change(&mut new);
        entities.insert(id.to_owned(), new.clone());
        let _ = self.pushes.send(Push::Set { id: id.to_owned(), old, new });
    }

    fn call_service(&self, domain: &str, service: &str, data: &Value, target: &Value) {
        self.calls.lock().unwrap().push((domain.to_owned(), service.to_owned(), data.clone()));
        if domain != "media_player" {
            return;
        }
        let ids: Vec<String> = match target.get("entity_id") {
            Some(Value::String(id)) => vec![id.clone()],
            Some(Value::Array(ids)) => ids.iter().filter_map(|i| i.as_str().map(str::to_owned)).collect(),
            _ => vec![],
        };
        for id in ids {
            self.update(&id, false, |e| match service {
                "volume_set" => {
                    e.attributes.insert("volume_level".into(), data["volume_level"].clone());
                }
                "volume_mute" => {
                    e.attributes.insert("is_volume_muted".into(), data["is_volume_muted"].clone());
                }
                "select_source" => {
                    e.attributes.insert("source".into(), data["source"].clone());
                }
                "turn_on" => e.state = "on".into(),
                "turn_off" => e.state = "off".into(),
                "volume_up" | "volume_down" => {
                    let step = if service == "volume_up" { 0.01 } else { -0.01 };
                    let level = e.attributes.get("volume_level").and_then(Value::as_f64).unwrap_or(0.0);
                    let level = ((level + step).clamp(0.0, 1.0) * 100.0).round() / 100.0;
                    e.attributes.insert("volume_level".into(), json!(level));
                }
                _ => {}
            });
        }
    }
}

fn full(e: &Entity) -> Value {
    json!({ "s": e.state, "a": e.attributes, "c": "01FAKE", "lc": 0.0, "lu": 0.0 })
}

/// The compressed event for a change, or `None` when nothing differs.
fn event_for(id: &str, old: &Option<Entity>, new: &Entity) -> Option<Value> {
    let Some(old) = old else { return Some(json!({ "a": { id: full(new) } })) };
    let mut plus = Map::new();
    if old.state != new.state {
        plus.insert("s".into(), json!(new.state));
    }
    let changed: Map<String, Value> = new.attributes.iter().filter(|(k, v)| old.attributes.get(*k) != Some(v)).map(|(k, v)| (k.clone(), v.clone())).collect();
    if !changed.is_empty() {
        plus.insert("a".into(), Value::Object(changed));
    }
    let removed: Vec<&String> = old.attributes.keys().filter(|k| !new.attributes.contains_key(*k)).collect();
    let mut diff = Map::new();
    if !plus.is_empty() {
        diff.insert("+".into(), Value::Object(plus));
    }
    if !removed.is_empty() {
        diff.insert("-".into(), json!({ "a": removed }));
    }
    (!diff.is_empty()).then(|| json!({ "c": { id: diff } }))
}

struct Subscription {
    id: u64,
    entities: BTreeSet<String>,
}

fn result(id: u64, outcome: Result<Value, (&str, &str)>) -> String {
    match outcome {
        Ok(result) => json!({ "id": id, "type": "result", "success": true, "result": result }),
        Err((code, message)) => json!({ "id": id, "type": "result", "success": false, "error": { "code": code, "message": message } }),
    }
    .to_string()
}

impl Shared {
    fn handle(&self, frame: &str, sub: &mut Option<Subscription>) -> Vec<String> {
        let Ok(msg) = serde_json::from_str::<Value>(frame) else { return vec![] };
        let Some(id) = msg["id"].as_u64() else { return vec![result(0, Err(("invalid_format", "Message incorrectly formatted.")))] };
        match msg["type"].as_str().unwrap_or_default() {
            "ping" => vec![json!({ "id": id, "type": "pong" }).to_string()],
            "get_states" => {
                let all: Vec<Value> = self.entities.lock().unwrap().iter().map(|(k, e)| json!({ "entity_id": k, "state": e.state, "attributes": e.attributes })).collect();
                vec![result(id, Ok(json!(all)))]
            }
            "subscribe_entities" => {
                let wanted: BTreeSet<String> = msg["entity_ids"].as_array().into_iter().flatten().filter_map(|i| i.as_str().map(str::to_owned)).collect();
                let entities = self.entities.lock().unwrap();
                let initial: Map<String, Value> = wanted.iter().filter_map(|k| Some((k.clone(), full(entities.get(k)?)))).collect();
                *sub = Some(Subscription { id, entities: wanted });
                vec![result(id, Ok(Value::Null)), json!({ "id": id, "type": "event", "event": { "a": initial } }).to_string()]
            }
            "unsubscribe_events" => {
                if sub.as_ref().is_some_and(|s| Some(s.id) == msg["subscription"].as_u64()) {
                    *sub = None;
                    vec![result(id, Ok(Value::Null))]
                } else {
                    vec![result(id, Err(("not_found", "Subscription not found.")))]
                }
            }
            "call_service" => {
                let (domain, service) = (msg["domain"].as_str().unwrap_or_default(), msg["service"].as_str().unwrap_or_default());
                self.call_service(domain, service, &msg["service_data"], &msg["target"]);
                vec![result(id, Ok(json!({ "context": { "id": "01FAKE" } })))]
            }
            _ => vec![result(id, Err(("unknown_command", "Unknown command.")))],
        }
    }
}

async fn serve(stream: TcpStream, shared: Arc<Shared>) {
    let Ok(socket) = tokio_tungstenite::accept_async(stream).await else { return };
    shared.connections.fetch_add(1, Ordering::SeqCst);
    let mut pushes = shared.pushes.subscribe();
    let (mut sink, mut stream) = socket.split();
    let hello = json!({ "type": "auth_required", "ha_version": "2024.11.3" }).to_string();
    if sink.send(Message::Text(hello)).await.is_err() {
        return;
    }
    let Some(Ok(Message::Text(auth))) = stream.next().await else { return };
    let auth: Value = serde_json::from_str(&auth).unwrap_or_default();
    if auth["type"] != "auth" || auth["access_token"] != shared.token.as_str() {
        let invalid = json!({ "type": "auth_invalid", "message": "Invalid access token or password" }).to_string();
        let _ = sink.send(Message::Text(invalid)).await;
        let _ = sink.close().await;
        return;
    }
    if sink.send(Message::Text(json!({ "type": "auth_ok", "ha_version": "2024.11.3" }).to_string())).await.is_err() {
        return;
    }
    let mut sub: Option<Subscription> = None;
    loop {
        let out = tokio::select! {
            frame = stream.next() => match frame {
                Some(Ok(Message::Text(text))) => shared.handle(&text, &mut sub),
                Some(Ok(Message::Close(_)) | Err(_)) | None => return,
                Some(Ok(_)) => continue,
            },
            push = pushes.recv() => match push {
                Ok(Push::Kick) => return,
                Ok(Push::Set { id, old, new }) => match &sub {
                    Some(s) if s.entities.contains(&id) => {
                        event_for(&id, &old, &new).map(|event| json!({ "id": s.id, "type": "event", "event": event }).to_string()).into_iter().collect()
                    }
                    _ => continue,
                },
                Err(_) => continue,
            },
        };
        for frame in out {
            if sink.send(Message::Text(frame)).await.is_err() {
                return;
            }
        }
    }
}
