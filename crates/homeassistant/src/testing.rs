//! `FakeHa`: an in-process Home Assistant WebSocket server for tests. It speaks the real protocol (auth, ping,
//! `get_states`, `subscribe_entities` with compressed diffs, `call_service`) and gives media players the effects
//! the services have in Home Assistant.

use crate::config::HaConfig;
use crate::protocol::{attribute, domain, field, service, state, MessageType};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use wsclient::testing::{Broadcaster, FakeServer, Peer, Reply};

const HA_VERSION: &str = "2024.11.3";
const FAKE_CONTEXT: &str = "01FAKE";
const VOLUME_STEP: f64 = 0.01;

#[derive(Clone, Debug, PartialEq)]
struct Entity {
    state: String,
    attributes: Map<String, Value>,
}

#[derive(Clone, Debug)]
struct Push {
    id: String,
    old: Option<Entity>,
    new: Entity,
}

struct Shared {
    token: String,
    entities: Mutex<BTreeMap<String, Entity>>,
    calls: Mutex<Vec<(String, String, Value)>>,
}

pub struct FakeHa {
    server: FakeServer<Push>,
    shared: Arc<Shared>,
}

impl FakeHa {
    pub async fn start(token: &str) -> Self {
        Self::start_on("127.0.0.1:0", token).await
    }

    /// On a given address, to run it as a stand-in Home Assistant for the desktop UI.
    pub async fn start_on(addr: &str, token: &str) -> Self {
        let shared = Arc::new(Shared { token: token.to_owned(), entities: Mutex::default(), calls: Mutex::default() });
        let server = FakeServer::start(addr, {
            let shared = shared.clone();
            move |pushes| HaPeer { shared: shared.clone(), pushes, signed_in: false, sub: None }
        })
        .await;
        Self { server, shared }
    }

    pub fn config(&self) -> HaConfig {
        HaConfig::new(format!("http://{}", self.addr()), self.shared.token.clone())
    }

    fn addr(&self) -> SocketAddr {
        self.server.addr()
    }

    /// Sets an entity (creating it if need be) and tells whoever subscribed. `attributes` is a JSON object.
    pub fn set_entity(&self, id: &str, state: &str, attributes: Value) {
        let attributes = attributes.as_object().cloned().unwrap_or_default();
        self.shared.update(id, true, |e| *e = Entity { state: state.to_owned(), attributes }, &self.server.broadcaster());
    }

    /// Service calls received so far: domain, service and `service_data`.
    pub fn calls(&self) -> Vec<(String, String, Value)> {
        self.shared.calls.lock().unwrap().clone()
    }

    /// Drops every connection.
    pub fn kick(&self) {
        self.server.kick();
    }

    /// Connections accepted so far.
    pub fn connections(&self) -> usize {
        self.server.accepted()
    }
}

impl Shared {
    fn update(&self, id: &str, create: bool, change: impl FnOnce(&mut Entity), to: &Broadcaster<Push>) {
        let mut entities = self.entities.lock().unwrap();
        let old = entities.get(id).cloned();
        if old.is_none() && !create {
            return;
        }
        let mut new = old.clone().unwrap_or(Entity { state: String::new(), attributes: Map::new() });
        change(&mut new);
        entities.insert(id.to_owned(), new.clone());
        to.send(Push { id: id.to_owned(), old, new });
    }

    fn call_service(&self, name: &str, service: &str, data: &Value, target: &Value, to: &Broadcaster<Push>) {
        self.calls.lock().unwrap().push((name.to_owned(), service.to_owned(), data.clone()));
        if name != domain::MEDIA_PLAYER {
            return;
        }
        let ids: Vec<String> = match target.get(field::ENTITY_ID) {
            Some(Value::String(id)) => vec![id.clone()],
            Some(Value::Array(ids)) => ids.iter().filter_map(|i| i.as_str().map(str::to_owned)).collect(),
            _ => vec![],
        };
        for id in ids {
            self.update(
                &id,
                false,
                |e| match service {
                    service::VOLUME_SET => {
                        e.attributes.insert(attribute::VOLUME_LEVEL.into(), data[attribute::VOLUME_LEVEL].clone());
                    }
                    service::VOLUME_MUTE => {
                        e.attributes.insert(attribute::IS_VOLUME_MUTED.into(), data[attribute::IS_VOLUME_MUTED].clone());
                    }
                    service::SELECT_SOURCE => {
                        e.attributes.insert(attribute::SOURCE.into(), data[attribute::SOURCE].clone());
                    }
                    service::TURN_ON => e.state = state::ON.into(),
                    service::TURN_OFF => e.state = state::OFF.into(),
                    service::VOLUME_UP | service::VOLUME_DOWN => {
                        let step = if service == service::VOLUME_UP { VOLUME_STEP } else { -VOLUME_STEP };
                        let level = e.attributes.get(attribute::VOLUME_LEVEL).and_then(Value::as_f64).unwrap_or(0.0);
                        let level = ((level + step).clamp(0.0, 1.0) * 100.0).round() / 100.0;
                        e.attributes.insert(attribute::VOLUME_LEVEL.into(), json!(level));
                    }
                    _ => {}
                },
                to,
            );
        }
    }
}

fn full(e: &Entity) -> Value {
    json!({ "s": e.state, "a": e.attributes, "c": FAKE_CONTEXT, "lc": 0.0, "lu": 0.0 })
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
    let kind = MessageType::Result.as_str();
    match outcome {
        Ok(result) => json!({ field::ID: id, field::TYPE: kind, field::SUCCESS: true, field::RESULT: result }),
        Err((code, message)) => json!({ field::ID: id, field::TYPE: kind, field::SUCCESS: false, field::ERROR: { field::CODE: code, field::MESSAGE: message } }),
    }
    .to_string()
}

fn message(kind: MessageType, rest: Value) -> String {
    let mut frame = json!({ field::TYPE: kind.as_str() });
    if let (Some(frame), Value::Object(rest)) = (frame.as_object_mut(), rest) {
        frame.extend(rest);
    }
    frame.to_string()
}

struct HaPeer {
    shared: Arc<Shared>,
    pushes: Broadcaster<Push>,
    signed_in: bool,
    sub: Option<Subscription>,
}

impl Peer<Push> for HaPeer {
    fn opened(&mut self) -> Reply {
        Reply::say(message(MessageType::AuthRequired, json!({ field::HA_VERSION: HA_VERSION })))
    }

    fn frame(&mut self, text: &str) -> Reply {
        if self.signed_in {
            return Reply::say_all(self.handle(text));
        }
        let auth: Value = serde_json::from_str(text).unwrap_or_default();
        if auth[field::TYPE] != MessageType::Auth.as_str() || auth[field::ACCESS_TOKEN] != self.shared.token.as_str() {
            return Reply::say(message(MessageType::AuthInvalid, json!({ field::MESSAGE: "Invalid access token or password" }))).then_close();
        }
        self.signed_in = true;
        Reply::say(message(MessageType::AuthOk, json!({ field::HA_VERSION: HA_VERSION })))
    }

    fn broadcast(&mut self, push: &Push) -> Reply {
        match &self.sub {
            Some(s) if s.entities.contains(&push.id) => {
                let event = event_for(&push.id, &push.old, &push.new);
                Reply::say_all(event.map(|event| event_frame(s.id, event)).into_iter().collect())
            }
            _ => Reply::nothing(),
        }
    }
}

fn event_frame(subscription: u64, event: Value) -> String {
    json!({ field::ID: subscription, field::TYPE: MessageType::Event.as_str(), field::EVENT: event }).to_string()
}

impl HaPeer {
    fn handle(&mut self, frame: &str) -> Vec<String> {
        let Ok(msg) = serde_json::from_str::<Value>(frame) else { return vec![] };
        let Some(id) = msg[field::ID].as_u64() else { return vec![result(0, Err(("invalid_format", "Message incorrectly formatted.")))] };
        let shared = &self.shared;
        match msg[field::TYPE].as_str().and_then(MessageType::parse) {
            Some(MessageType::Ping) => vec![json!({ field::ID: id, field::TYPE: MessageType::Pong.as_str() }).to_string()],
            Some(MessageType::GetStates) => {
                let all: Vec<Value> = shared
                    .entities
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|(k, e)| json!({ field::ENTITY_ID: k, field::STATE: e.state, field::ATTRIBUTES: e.attributes }))
                    .collect();
                vec![result(id, Ok(json!(all)))]
            }
            Some(MessageType::SubscribeEntities) => {
                let wanted: BTreeSet<String> = msg[field::ENTITY_IDS].as_array().into_iter().flatten().filter_map(|i| i.as_str().map(str::to_owned)).collect();
                let entities = shared.entities.lock().unwrap();
                let initial: Map<String, Value> = wanted.iter().filter_map(|k| Some((k.clone(), full(entities.get(k)?)))).collect();
                self.sub = Some(Subscription { id, entities: wanted });
                vec![result(id, Ok(Value::Null)), event_frame(id, json!({ "a": initial }))]
            }
            Some(MessageType::UnsubscribeEvents) => {
                if self.sub.as_ref().is_some_and(|s| Some(s.id) == msg[field::SUBSCRIPTION].as_u64()) {
                    self.sub = None;
                    vec![result(id, Ok(Value::Null))]
                } else {
                    vec![result(id, Err(("not_found", "Subscription not found.")))]
                }
            }
            Some(MessageType::CallService) => {
                let (name, service) = (msg[field::DOMAIN].as_str().unwrap_or_default(), msg[field::SERVICE].as_str().unwrap_or_default());
                shared.call_service(name, service, &msg[field::SERVICE_DATA], &msg[field::TARGET], &self.pushes);
                vec![result(id, Ok(json!({ "context": { "id": FAKE_CONTEXT } })))]
            }
            _ => vec![result(id, Err(("unknown_command", "Unknown command.")))],
        }
    }
}
