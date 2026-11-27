//! Home Assistant's frames: the ones it sends, sorted, and the compressed entity events.

use crate::error::{Error, Result};
use crate::ids::EntityId;
use crate::protocol::{field, MessageType};
use crate::state::EntityState;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[derive(Debug)]
pub(crate) enum Incoming {
    AuthRequired,
    AuthOk { version: Option<String> },
    AuthInvalid { message: String },
    Reply { id: u32, outcome: Result<Value> },
    Event { id: u32, event: Value },
    Other,
}

impl Incoming {
    pub(crate) fn parse(frame: &str) -> Result<Self> {
        let v: Value = serde_json::from_str(frame).map_err(|e| Error::Decode(e.to_string()))?;
        let text = |key: &str| v.get(key).and_then(Value::as_str).map(str::to_owned);
        let id = v.get(field::ID).and_then(Value::as_u64).map(|n| n as u32);
        let kind = v.get(field::TYPE).and_then(Value::as_str).and_then(MessageType::parse);
        Ok(match (kind, id) {
            (Some(MessageType::AuthRequired), _) => Self::AuthRequired,
            (Some(MessageType::AuthOk), _) => Self::AuthOk { version: text(field::HA_VERSION) },
            (Some(MessageType::AuthInvalid), _) => Self::AuthInvalid { message: text(field::MESSAGE).unwrap_or_default() },
            (Some(MessageType::Result), Some(id)) => {
                let outcome = if v.get(field::SUCCESS).and_then(Value::as_bool) == Some(true) {
                    Ok(v.get(field::RESULT).cloned().unwrap_or(Value::Null))
                } else {
                    let error = v.get(field::ERROR);
                    let text = |key: &str| error.and_then(|e| e.get(key)).and_then(Value::as_str).unwrap_or_default().to_owned();
                    Err(Error::Failed { code: text(field::CODE), message: text(field::MESSAGE) })
                };
                Self::Reply { id, outcome }
            }
            (Some(MessageType::Event), Some(id)) => Self::Event { id, event: v.get(field::EVENT).cloned().unwrap_or(Value::Null) },
            _ => Self::Other,
        })
    }
}

/// One `subscribe_entities` event. Added entities arrive whole, changed ones as a diff, removed ones as ids.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Compressed {
    #[serde(default)]
    pub a: BTreeMap<EntityId, Added>,
    #[serde(default)]
    pub c: BTreeMap<EntityId, Changed>,
    #[serde(default)]
    pub r: Vec<EntityId>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Added {
    #[serde(default)]
    pub s: String,
    #[serde(default)]
    pub a: Map<String, Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Changed {
    #[serde(rename = "+")]
    pub plus: Option<Plus>,
    #[serde(rename = "-")]
    pub minus: Option<Minus>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Plus {
    pub s: Option<String>,
    /// Only the attributes that changed.
    pub a: Option<Map<String, Value>>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Minus {
    /// Attributes that are gone.
    #[serde(default)]
    pub a: Vec<String>,
}

/// The reply to `get_states`.
pub(crate) fn states(result: Value) -> Result<Vec<(EntityId, EntityState)>> {
    #[derive(Deserialize)]
    struct Full {
        entity_id: EntityId,
        state: String,
        #[serde(default)]
        attributes: Map<String, Value>,
    }
    let all: Vec<Full> = serde_json::from_value(result).map_err(|e| Error::Decode(e.to_string()))?;
    Ok(all.into_iter().map(|f| (f.entity_id, EntityState { state: f.state, attributes: f.attributes })).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_sorted() {
        let parse = |s: &str| Incoming::parse(s).unwrap();
        assert!(matches!(parse(r#"{"type":"auth_required","ha_version":"2024.11.3"}"#), Incoming::AuthRequired));
        assert!(matches!(parse(r#"{"type":"auth_ok","ha_version":"2024.11.3"}"#), Incoming::AuthOk { version: Some(v) } if v == "2024.11.3"));
        assert!(matches!(parse(r#"{"type":"auth_invalid","message":"Invalid access token or password"}"#), Incoming::AuthInvalid { .. }));
        assert!(matches!(parse(r#"{"id":3,"type":"result","success":true,"result":null}"#), Incoming::Reply { id: 3, outcome: Ok(_) }));
        let failed = r#"{"id":4,"type":"result","success":false,"error":{"code":"not_found","message":"Service not found."}}"#;
        let Incoming::Reply { id: 4, outcome: Err(e) } = parse(failed) else { panic!() };
        assert_eq!(e, Error::Failed { code: "not_found".into(), message: "Service not found.".into() });
        assert!(matches!(parse(r#"{"id":1,"type":"event","event":{"a":{}}}"#), Incoming::Event { id: 1, .. }));
        assert!(matches!(parse(r#"{"id":5,"type":"pong"}"#), Incoming::Other));
        assert!(Incoming::parse("nope").is_err());
    }

    #[test]
    fn compressed_event_fields() {
        let c: Compressed = serde_json::from_str(
            r#"{"c":{"media_player.theater_receiver":{"+":{"s":"off","a":{"volume_level":0.4},"c":"01J","lu":1.5},"-":{"a":["source"]}}},"r":["light.old"]}"#,
        )
        .unwrap();
        let change = &c.c[&EntityId::from("media_player.theater_receiver")];
        assert_eq!(change.plus.as_ref().unwrap().s.as_deref(), Some("off"));
        assert_eq!(change.minus.as_ref().unwrap().a, ["source"]);
        assert_eq!(c.r, [EntityId::from("light.old")]);
    }
}
