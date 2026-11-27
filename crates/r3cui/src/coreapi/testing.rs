//! A fake core for tests: it signs a client in by its token and answers the requests the UI makes, from state the
//! test can read and change.

use crate::coreapi::messages;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use wsclient::testing::{FakeServer, Peer, Reply};

pub struct FakeState {
    pub token: String,
    pub version: Value,
    pub installed_firmware: String,
    /// The update the core offers, if any.
    pub update: Option<Value>,
    pub update_downloaded: bool,
    pub battery_ok: bool,
    pub web_enabled: bool,
    pub web_pin: Option<String>,
    /// What `set_ui_info` was last told.
    pub ui_info: Option<Value>,
    /// Whether the last `check_system_update` forced a look.
    pub forced: Option<bool>,
    pub update_requests: u32,
    pub voice_cfg: Value,
    pub voice_assistants: Vec<Value>,
    /// Each `execute_entity_command` received, in order.
    pub entity_commands: Vec<EntityCommand>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntityCommand {
    pub entity_id: String,
    pub cmd_id: String,
    pub params: Option<Value>,
}

pub type Shared = Arc<Mutex<FakeState>>;

impl FakeState {
    pub fn new(token: &str) -> Self {
        Self {
            token: token.into(),
            version: json!({"model": "UCR3", "device_name": "Remote", "hostname": "UC-Remote-3", "address": "192.168.1.40", "api": "0.50.5", "core": "0.60.0", "ui": "0.1.0", "os": "2.8.4"}),
            installed_firmware: "2.8.4".into(),
            update: Some(json!({
                "id": "u-2.9.0", "title": {"en": "Firmware 2.9.0"}, "description": {"en": "Faster Wi-Fi."}, "version": "2.9.0",
                "channel": "STABLE", "release_date": "2026-11-10T00:00:00Z", "size": 120_000_000u64, "download": "PENDING",
            })),
            update_downloaded: false,
            battery_ok: true,
            web_enabled: false,
            web_pin: None,
            ui_info: None,
            forced: None,
            update_requests: 0,
            voice_cfg: json!({"microphone": true, "voice_assistant": {"active": voice_assistant(), "profile_id": "p1", "speech_response": false}}),
            voice_assistants: vec![voice_assistant()],
            entity_commands: Vec::new(),
        }
    }

    fn update_info(&self) -> Value {
        let available: Vec<Value> = self
            .update
            .iter()
            .map(|u| {
                let mut u = u.clone();
                u["download"] = json!(if self.update_downloaded { "DOWNLOADED" } else { "PENDING" });
                u
            })
            .collect();
        json!({"update_in_progress": false, "last_check_date": "2026-11-26T10:00:00Z", "installed_version": self.installed_firmware, "available": available})
    }
}

fn voice_assistant() -> Value {
    json!({
        "entity_id": "r3cui.voice", "name": {"en": "r3cui voice"}, "state": "ON", "features": ["transcription", "response_text"],
        "profiles": [{"id": "p1", "name": "English", "language": "en"}], "preferred_profile": "p1",
    })
}

pub struct FakeCore {
    shared: Shared,
    signed_in: bool,
}

impl FakeCore {
    pub fn new(shared: Shared) -> Self {
        Self { shared, signed_in: false }
    }

    /// A listening fake core and its state.
    pub async fn start(token: &str) -> (FakeServer<Value>, Shared) {
        let shared: Shared = Arc::new(Mutex::new(FakeState::new(token)));
        let server = FakeServer::start("127.0.0.1:0", {
            let shared = shared.clone();
            move |_| FakeCore::new(shared.clone())
        })
        .await;
        (server, shared)
    }
}

impl Peer<Value> for FakeCore {
    fn opened(&mut self) -> Reply {
        Reply::say(json!({"kind": "event", "msg": messages::AUTH_REQUIRED}).to_string())
    }

    /// A test's `FakeServer::broadcast(msg_data)` reaches the client as an `assistant_event`.
    fn broadcast(&mut self, data: &Value) -> Reply {
        Reply::say(json!({"kind": "event", "msg": messages::ASSISTANT_EVENT, "cat": "ENTITY", "msg_data": data}).to_string())
    }

    fn frame(&mut self, text: &str) -> Reply {
        let req: Value = serde_json::from_str(text).unwrap();
        let answer = |code: i64, data: Value| json!({"kind": "resp", "req_id": req["id"], "msg": req["msg"], "code": code, "msg_data": data}).to_string();
        let mut state = self.shared.lock().unwrap();
        if !self.signed_in {
            let good = req["msg_data"]["token"] == state.token.as_str();
            self.signed_in = good;
            let frame = json!({"kind": "resp", "req_id": req["id"], "msg": messages::AUTHENTICATION, "code": if good { 200 } else { 401 }}).to_string();
            return if good { Reply::say(frame) } else { Reply::say(frame).then_close() };
        }
        let data = &req["msg_data"];
        match req["msg"].as_str() {
            Some(messages::SUBSCRIBE_EVENTS) => Reply::nothing(),
            Some(messages::VERSION) => {
                let battery = json!({"kind": "event", "msg": messages::BATTERY_STATUS, "msg_data": {"capacity": 80}}).to_string();
                Reply::say_all(vec![battery, answer(200, state.version.clone())])
            }
            Some(messages::SET_UI_INFO) => {
                state.ui_info = Some(data.clone());
                Reply::say(answer(200, json!({})))
            }
            Some(messages::CHECK_SYSTEM_UPDATE) => {
                state.forced = data["force_update"].as_bool();
                Reply::say(answer(200, state.update_info()))
            }
            Some(messages::UPDATE_SYSTEM) => {
                state.update_requests += 1;
                if !state.battery_ok {
                    return Reply::say(answer(503, json!({"code": 503, "message": "battery too low"})));
                }
                let began = if state.update_downloaded { "START" } else { "DOWNLOAD" };
                state.update_downloaded = true;
                let event = json!({"kind": "event", "msg": messages::SOFTWARE_UPDATE, "msg_data": {"event_type": "START", "update_id": data["update_id"]}});
                Reply::say_all(vec![answer(200, json!({"state": began})), event.to_string()])
            }
            Some(messages::GET_API_ACCESS) => Reply::say(answer(200, json!({"web_configurator": {"enabled": state.web_enabled}}))),
            Some(messages::SET_API_ACCESS) => {
                let web = &data["web_configurator"];
                state.web_enabled = web["enabled"].as_bool().unwrap_or(false);
                if state.web_enabled {
                    state.web_pin = web["pin"].as_str().map(String::from);
                }
                Reply::say(answer(200, json!({"web_configurator": {"enabled": state.web_enabled}})))
            }
            Some(messages::GET_VOICE_CONTROL_CFG) => Reply::say(answer(200, state.voice_cfg.clone())),
            Some(messages::SET_VOICE_CONTROL_CFG) => {
                if let Some(mic) = data["microphone"].as_bool() {
                    state.voice_cfg["microphone"] = json!(mic);
                }
                let wanted = &data["voice_assistant"];
                if wanted.is_object() {
                    let active = state.voice_assistants.iter().find(|a| a["entity_id"] == wanted["entity_id"]).cloned();
                    let target = &mut state.voice_cfg["voice_assistant"];
                    target["active"] = active.unwrap_or(Value::Null);
                    for key in ["profile_id", "speech_response"] {
                        if !wanted[key].is_null() {
                            target[key] = wanted[key].clone();
                        }
                    }
                }
                Reply::say(answer(200, state.voice_cfg.clone()))
            }
            Some(messages::GET_VOICE_ASSISTANTS) => Reply::say(answer(200, json!({"entities": state.voice_assistants}))),
            Some(messages::EXECUTE_ENTITY_COMMAND) => {
                state.entity_commands.push(EntityCommand {
                    entity_id: data["entity_id"].as_str().unwrap_or_default().to_string(),
                    cmd_id: data["cmd_id"].as_str().unwrap_or_default().to_string(),
                    params: data.get("params").cloned(),
                });
                Reply::say(answer(200, json!({})))
            }
            _ => Reply::say(answer(404, json!({"message": "nope"}))),
        }
    }
}
