//! The Integration-API as the relay speaks it: message names, the envelope, and the few payloads it sends.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;
use voicelink::{ErrorCode, SessionId};

pub const ENTITY_ID: &str = voicelink::RELAY_ENTITY_ID;
pub const DRIVER_NAME: &str = "r3cui-voice";
pub const API_VERSION: &str = "0.20.0";
pub const DRIVER_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DRIVER_JSON: &str = include_str!("../driver.json");

pub mod msg {
    pub const AUTHENTICATION: &str = "authentication";
    pub const GET_DRIVER_VERSION: &str = "get_driver_version";
    pub const DRIVER_VERSION: &str = "driver_version";
    pub const GET_DRIVER_METADATA: &str = "get_driver_metadata";
    pub const DRIVER_METADATA: &str = "driver_metadata";
    pub const GET_DEVICE_STATE: &str = "get_device_state";
    pub const DEVICE_STATE: &str = "device_state";
    pub const GET_AVAILABLE_ENTITIES: &str = "get_available_entities";
    pub const AVAILABLE_ENTITIES: &str = "available_entities";
    pub const GET_ENTITY_STATES: &str = "get_entity_states";
    pub const ENTITY_STATES: &str = "entity_states";
    pub const SUBSCRIBE_EVENTS: &str = "subscribe_events";
    pub const UNSUBSCRIBE_EVENTS: &str = "unsubscribe_events";
    pub const SETUP_DRIVER: &str = "setup_driver";
    pub const DRIVER_SETUP_CHANGE: &str = "driver_setup_change";
    pub const ENTITY_COMMAND: &str = "entity_command";
    pub const ENTITY_CHANGE: &str = "entity_change";
    pub const ASSISTANT_EVENT: &str = "assistant_event";
    pub const RESULT: &str = "result";
    pub const CONNECT: &str = "connect";
    pub const DISCONNECT: &str = "disconnect";
    pub const ENTER_STANDBY: &str = "enter_standby";
    pub const EXIT_STANDBY: &str = "exit_standby";
}

pub mod kind {
    pub const REQ: &str = "req";
    pub const RESP: &str = "resp";
    pub const EVENT: &str = "event";
}

pub mod cat {
    pub const DEVICE: &str = "DEVICE";
    pub const ENTITY: &str = "ENTITY";
}

pub mod cmd {
    pub const VOICE_START: &str = "voice_start";
    pub const VOICE_END: &str = "voice_end";
}

pub mod code {
    pub const OK: u16 = 200;
    pub const BAD_REQUEST: u16 = 400;
    pub const NOT_FOUND: u16 = 404;
    pub const NOT_IMPLEMENTED: u16 = 501;
}

pub const ENTITY_TYPE_VOICE_ASSISTANT: &str = "voice_assistant";

#[derive(Debug, Deserialize)]
pub struct Incoming {
    pub kind: String,
    #[serde(default)]
    pub id: u64,
    pub msg: String,
    #[serde(default)]
    pub msg_data: Value,
}

#[derive(Debug, Deserialize)]
pub struct EntityCommand {
    pub entity_id: String,
    pub cmd_id: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum DeviceState {
    Connected,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum EntityState {
    On,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Feature {
    Transcription,
    ResponseText,
}

#[derive(Serialize)]
struct Entity {
    entity_id: &'static str,
    entity_type: &'static str,
    name: Value,
    features: [Feature; 2],
    options: Value,
}

pub fn entity() -> Value {
    let entity = Entity {
        entity_id: ENTITY_ID,
        entity_type: ENTITY_TYPE_VOICE_ASSISTANT,
        name: json!({ "en": "r3cui" }),
        features: [Feature::Transcription, Feature::ResponseText],
        options: json!({
            "audio_cfg": { "channels": 1, "sample_rate": 16000, "sample_format": "I16" },
            "profiles": [{ "id": "r3cui", "name": "r3cui" }],
        }),
    };
    serde_json::to_value(entity).expect("entity serialises")
}

pub fn entity_attributes() -> Value {
    json!({ "state": EntityState::On })
}

pub fn driver_version() -> Value {
    json!({ "name": DRIVER_NAME, "version": { "api": API_VERSION, "driver": DRIVER_VERSION } })
}

pub fn driver_metadata() -> Value {
    serde_json::from_str(DRIVER_JSON).expect("driver.json is JSON")
}

#[derive(Serialize)]
struct Response<'a> {
    kind: &'static str,
    req_id: u64,
    msg: &'a str,
    code: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    msg_data: Option<Value>,
}

pub fn response(req_id: u64, code: u16, msg: &str, data: Option<Value>) -> Message {
    text(&Response {
        kind: kind::RESP,
        req_id,
        msg,
        code,
        msg_data: data,
    })
}

pub fn result(req_id: u64, code: u16) -> Message {
    response(req_id, code, msg::RESULT, None)
}

#[derive(Serialize)]
struct Event<'a, T> {
    kind: &'static str,
    msg: &'a str,
    cat: &'a str,
    msg_data: T,
}

fn event(msg: &str, cat: &str, data: impl Serialize) -> Message {
    text(&Event {
        kind: kind::EVENT,
        msg,
        cat,
        msg_data: data,
    })
}

pub fn device_state(state: DeviceState) -> Message {
    event(msg::DEVICE_STATE, cat::DEVICE, json!({ "state": state }))
}

pub fn setup_finished() -> Message {
    event(
        msg::DRIVER_SETUP_CHANGE,
        cat::DEVICE,
        json!({ "event_type": "STOP", "state": "OK" }),
    )
}

pub fn entity_change() -> Message {
    event(
        msg::ENTITY_CHANGE,
        cat::ENTITY,
        json!({ "entity_type": ENTITY_TYPE_VOICE_ASSISTANT, "entity_id": ENTITY_ID, "attributes": entity_attributes() }),
    )
}

pub fn entity_states() -> Value {
    json!([{ "entity_type": ENTITY_TYPE_VOICE_ASSISTANT, "entity_id": ENTITY_ID, "attributes": entity_attributes() }])
}

#[derive(Serialize)]
pub struct ErrorData {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Assistant {
    Ready,
    Finished,
    Error { data: ErrorData },
}

impl Assistant {
    pub fn error(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::Error {
            data: ErrorData {
                code,
                message: message.into(),
            },
        }
    }
}

#[derive(Serialize)]
struct AssistantEventData {
    entity_id: &'static str,
    session_id: SessionId,
    #[serde(flatten)]
    event: Assistant,
}

pub fn assistant_event(session_id: SessionId, event_data: Assistant) -> Message {
    event(
        msg::ASSISTANT_EVENT,
        cat::ENTITY,
        AssistantEventData {
            entity_id: ENTITY_ID,
            session_id,
            event: event_data,
        },
    )
}

fn text(value: &impl Serialize) -> Message {
    Message::Text(serde_json::to_string(value).expect("message serialises"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json_of(message: Message) -> Value {
        let Message::Text(t) = message else {
            panic!("not text")
        };
        serde_json::from_str(&t).unwrap()
    }

    #[test]
    fn ready_event_has_the_shape_of_the_spec() {
        assert_eq!(
            json_of(assistant_event(8, Assistant::Ready)),
            json!({"kind":"event","msg":"assistant_event","cat":"ENTITY","msg_data":{"type":"ready","entity_id":"voice","session_id":8}})
        );
    }

    #[test]
    fn error_event_has_the_shape_of_the_spec() {
        assert_eq!(
            json_of(assistant_event(
                8,
                Assistant::error(ErrorCode::NoTextRecognized, "I did not understand")
            )),
            json!({"kind":"event","msg":"assistant_event","cat":"ENTITY","msg_data":{
                "type":"error","entity_id":"voice","session_id":8,
                "data":{"code":"NO_TEXT_RECOGNIZED","message":"I did not understand"}}})
        );
    }

    #[test]
    fn finished_event_has_no_data() {
        let v = json_of(assistant_event(8, Assistant::Finished));
        assert_eq!(
            v["msg_data"],
            json!({"type":"finished","entity_id":"voice","session_id":8})
        );
    }

    #[test]
    fn result_has_no_payload() {
        assert_eq!(
            json_of(result(123, code::OK)),
            json!({"kind":"resp","req_id":123,"msg":"result","code":200})
        );
    }

    #[test]
    fn the_entity_is_a_transcribing_voice_assistant() {
        let e = entity();
        assert_eq!(e["entity_type"], "voice_assistant");
        assert_eq!(e["features"], json!(["transcription", "response_text"]));
        assert_eq!(e["options"]["audio_cfg"]["sample_format"], "I16");
    }

    #[test]
    fn driver_json_matches_the_crate() {
        let meta = driver_metadata();
        assert_eq!(meta["version"], DRIVER_VERSION);
        assert_eq!(meta["driver_id"], voicelink::RELAY_DRIVER_ID);
        assert!(meta.get("setup_data_schema").is_none());
    }
}
