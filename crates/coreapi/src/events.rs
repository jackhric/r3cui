//! The core's events the UI acts on, typed after `core-api`'s AsyncAPI spec (`components/messages`). A value the
//! spec doesn't list (a newer core) becomes `Unknown` rather than failing the event.

use serde::Deserialize;
use serde_json::Value;

use crate::system::SoftwareUpdate;
use crate::voice::EntityId;
use crate::{messages, Event};

#[derive(Debug, Clone, PartialEq)]
pub enum CoreEvent {
    PowerModeChange(PowerModeChange),
    BatteryStatus(BatteryStatus),
    WifiChange(WifiChange),
    SoftwareUpdate(SoftwareUpdate),
    Assistant(AssistantEvent),
    /// Any other event, or one whose payload didn't match.
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PowerMode {
    Normal,
    Idle,
    LowPower,
    Suspend,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PowerModeChange {
    pub mode: PowerMode,
    /// Not in the spec, but sent by some firmware: whether the remote sits on its charger.
    #[serde(default)]
    pub power_supply: Option<bool>,
    #[serde(default)]
    pub battery: Option<BatteryPower>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BatteryPower {
    #[serde(default)]
    pub power_supply: Option<bool>,
}

impl PowerModeChange {
    pub fn on_charger(&self) -> Option<bool> {
        self.power_supply.or(self.battery.as_ref().and_then(|b| b.power_supply))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PowerStatus {
    Charging,
    Discharging,
    NotCharging,
    Full,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BatteryStatus {
    pub capacity: u8,
    #[serde(default)]
    pub status: Option<PowerStatus>,
    #[serde(default)]
    pub power_supply: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WifiEvent {
    Connected,
    Disconnected,
    ScanStarted,
    ScanCompleted,
    ScanFailed,
    NetworkNotFound,
    WrongKey,
    NetworkAdded,
    NetworkRemoved,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct WifiChange {
    pub event: WifiEvent,
    /// The network the event is about, when it concerns one.
    #[serde(default)]
    pub id: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AssistantErrorCode {
    ServiceUnavailable,
    InvalidAudio,
    NoTextRecognized,
    IntentFailed,
    TtsFailed,
    Timeout,
    UnexpectedError,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum AssistantEventKind {
    /// The assistant waits for the audio.
    Ready,
    SttResponse { text: String },
    TextResponse { success: bool, text: String },
    SpeechResponse { url: String, mime_type: String },
    Finished,
    Error { code: AssistantErrorCode, message: String },
}

/// The event `assistant_event`: how a voice session is going.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AssistantEvent {
    pub entity_id: EntityId,
    pub session_id: u32,
    #[serde(flatten)]
    pub kind: AssistantEventKind,
}

impl Event {
    /// The event as the UI understands it.
    pub fn typed(&self) -> CoreEvent {
        fn parse<T: for<'de> Deserialize<'de>>(data: &Value, wrap: fn(T) -> CoreEvent) -> CoreEvent {
            match serde_json::from_value(data.clone()) {
                Ok(v) => wrap(v),
                Err(e) => {
                    tracing::debug!("unexpected event payload: {e}");
                    CoreEvent::Other
                }
            }
        }
        match self.msg.as_str() {
            messages::POWER_MODE_CHANGE => parse(&self.data, CoreEvent::PowerModeChange),
            messages::BATTERY_STATUS => parse(&self.data, CoreEvent::BatteryStatus),
            messages::WIFI_CHANGE => parse(&self.data, CoreEvent::WifiChange),
            messages::SOFTWARE_UPDATE => parse(&self.data, CoreEvent::SoftwareUpdate),
            messages::ASSISTANT_EVENT => parse(&self.data, CoreEvent::Assistant),
            _ => CoreEvent::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(msg: &str, data: Value) -> Event {
        Event { msg: msg.into(), cat: None, data }
    }

    #[test]
    fn power_and_battery() {
        let e = event("power_mode_change", json!({ "mode": "NORMAL", "wakeup": true }));
        assert_eq!(e.typed(), CoreEvent::PowerModeChange(PowerModeChange { mode: PowerMode::Normal, power_supply: None, battery: None }));
        let e = event("power_mode_change", json!({ "mode": "DEEP_NAP", "battery": { "power_supply": true } }));
        let CoreEvent::PowerModeChange(p) = e.typed() else { panic!() };
        assert_eq!((p.mode, p.on_charger()), (PowerMode::Unknown, Some(true)));
        let e = event("battery_status", json!({ "capacity": 81, "status": "CHARGING", "power_supply": true }));
        assert_eq!(e.typed(), CoreEvent::BatteryStatus(BatteryStatus { capacity: 81, status: Some(PowerStatus::Charging), power_supply: Some(true) }));
    }

    #[test]
    fn wifi() {
        let e = event("wifi_change", json!({ "event": "NETWORK_NOT_FOUND", "id": 3 }));
        assert_eq!(e.typed(), CoreEvent::WifiChange(WifiChange { event: WifiEvent::NetworkNotFound, id: Some(3) }));
        assert_eq!(event("wifi_change", json!({ "event": "SCAN_COMPLETED" })).typed(), CoreEvent::WifiChange(WifiChange { event: WifiEvent::ScanCompleted, id: None }));
    }

    #[test]
    fn assistant_events_of_every_type() {
        let parsed = |data: Value| {
            let CoreEvent::Assistant(a) = event("assistant_event", data).typed() else { panic!("not an assistant event") };
            assert_eq!((a.entity_id.0.as_str(), a.session_id), ("r3cui.voice", 7));
            a.kind
        };
        let base = |kind: &str| json!({ "type": kind, "entity_id": "r3cui.voice", "session_id": 7 });
        let with = |kind: &str, data: Value| {
            let mut v = base(kind);
            v["data"] = data;
            v
        };
        assert_eq!(parsed(base("ready")), AssistantEventKind::Ready);
        assert_eq!(parsed(with("stt_response", json!({ "text": "lights on" }))), AssistantEventKind::SttResponse { text: "lights on".into() });
        assert_eq!(
            parsed(with("text_response", json!({ "success": true, "text": "Done" }))),
            AssistantEventKind::TextResponse { success: true, text: "Done".into() }
        );
        assert_eq!(
            parsed(with("speech_response", json!({ "url": "http://x/a.mp3", "mime_type": "audio/mpeg" }))),
            AssistantEventKind::SpeechResponse { url: "http://x/a.mp3".into(), mime_type: "audio/mpeg".into() }
        );
        assert_eq!(parsed(base("finished")), AssistantEventKind::Finished);
        assert_eq!(
            parsed(with("error", json!({ "code": "NO_TEXT_RECOGNIZED", "message": "nothing heard" }))),
            AssistantEventKind::Error { code: AssistantErrorCode::NoTextRecognized, message: "nothing heard".into() }
        );
        let AssistantEventKind::Error { code, .. } = parsed(with("error", json!({ "code": "NEW_ONE", "message": "" }))) else { panic!() };
        assert_eq!(code, AssistantErrorCode::Unknown);
    }

    #[test]
    fn a_malformed_or_unknown_assistant_event_is_other() {
        assert_eq!(event("assistant_event", json!({ "type": "stt_response", "entity_id": "a.b", "session_id": 1 })).typed(), CoreEvent::Other);
        assert_eq!(event("assistant_event", json!({ "type": "dance", "entity_id": "a.b", "session_id": 1 })).typed(), CoreEvent::Other);
    }

    #[test]
    fn anything_else_is_other() {
        assert_eq!(event("battery_status", json!({ "capacity": "full" })).typed(), CoreEvent::Other);
        assert_eq!(event("software_update", json!({})).typed(), CoreEvent::Other);
    }
}
