//! Voice control: the settings, the voice assistants the core knows, and a voice session's commands, typed after
//! `core-api`'s AsyncAPI spec and `doc/entities/entity_voice_assistant.md`. A session starts with `voice_start`,
//! then `assistant_event`s (in `events`) report it; `voice_end` stops the audio.

use crate::system::{CallError, LangMap, Request};
use crate::{messages, Core};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// How long the core waits on the assistant before it gives up, in seconds. The stock UI sends this when it has no
/// other value.
pub const DEFAULT_VOICE_TIMEOUT: u32 = 15;

/// A core entity id, `<integration_id>.<entity_id>`, unique over all integrations.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntityId(pub String);

impl EntityId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl std::fmt::Display for EntityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// `execute_entity_command`: any entity command, with its `params` when it has some.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExecuteEntityCommand {
    pub entity_id: EntityId,
    pub cmd_id: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl Request for ExecuteEntityCommand {
    const MSG: &'static str = messages::EXECUTE_ENTITY_COMMAND;
    type Reply = Value;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceFeature {
    Transcription,
    ResponseText,
    ResponseSpeech,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct VoiceProfile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub language: Option<String>,
    /// Overrides the assistant's features when present.
    #[serde(default)]
    pub features: Option<Vec<VoiceFeature>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct VoiceAssistant {
    pub entity_id: EntityId,
    #[serde(default)]
    pub name: LangMap,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub features: Vec<VoiceFeature>,
    #[serde(default)]
    pub profiles: Vec<VoiceProfile>,
    #[serde(default)]
    pub preferred_profile: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CfgVoiceAssistant {
    /// Absent when no voice assistant is set.
    #[serde(default)]
    pub active: Option<VoiceAssistant>,
    #[serde(default)]
    pub profile_id: Option<String>,
    #[serde(default)]
    pub speech_response: bool,
}

/// `voice_control_cfg`, the answer to `get_voice_control_cfg` and `set_voice_control_cfg`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct VoiceControlCfg {
    pub microphone: bool,
    pub voice_assistant: CfgVoiceAssistant,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct GetVoiceControlCfg {}

impl Request for GetVoiceControlCfg {
    const MSG: &'static str = messages::GET_VOICE_CONTROL_CFG;
    type Reply = VoiceControlCfg;
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VoiceAssistantUpdate {
    /// Empty removes the configured voice assistant.
    pub entity_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speech_response: Option<bool>,
}

/// `set_voice_control_cfg`: a field left out keeps its value.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct VoiceControlUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub microphone: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_assistant: Option<VoiceAssistantUpdate>,
}

impl Request for VoiceControlUpdate {
    const MSG: &'static str = messages::SET_VOICE_CONTROL_CFG;
    type Reply = VoiceControlCfg;
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct GetVoiceAssistants {}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct VoiceAssistants {
    pub entities: Vec<VoiceAssistant>,
}

impl Request for GetVoiceAssistants {
    const MSG: &'static str = messages::GET_VOICE_ASSISTANTS;
    type Reply = VoiceAssistants;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AudioCfg {
    pub channels: u8,
    pub sample_rate: u32,
    pub sample_format: &'static str,
}

/// The parameters of `voice_start`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VoiceStart {
    /// Names the session in the audio and in every `assistant_event` about it.
    pub session_id: u32,
    pub timeout: u32,
    pub speech_response: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    /// The core's default (PCM, 16 kHz, mono, signed 16-bit) when left out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_cfg: Option<AudioCfg>,
}

impl VoiceStart {
    pub fn new(session_id: u32) -> Self {
        Self { session_id, timeout: DEFAULT_VOICE_TIMEOUT, speech_response: false, profile_id: None, audio_cfg: None }
    }
}

impl Core {
    pub async fn get_voice_control_cfg(&self) -> Result<VoiceControlCfg, CallError> {
        self.call(&GetVoiceControlCfg {}).await
    }

    pub async fn get_voice_assistants(&self) -> Result<Vec<VoiceAssistant>, CallError> {
        Ok(self.call(&GetVoiceAssistants {}).await?.entities)
    }

    pub async fn set_voice_control_cfg(&self, update: &VoiceControlUpdate) -> Result<VoiceControlCfg, CallError> {
        self.call(update).await
    }

    pub async fn voice_start(&self, entity_id: &EntityId, start: &VoiceStart) -> Result<(), CallError> {
        let params = serde_json::to_value(start).map_err(|e| CallError::Lost(e.to_string()))?;
        let command = ExecuteEntityCommand { entity_id: entity_id.clone(), cmd_id: messages::VOICE_START, params: Some(params) };
        self.call(&command).await.map(drop)
    }

    /// The stock UI sends `voice_end` with no parameters, so `session_id` is only sent when given.
    pub async fn voice_end(&self, entity_id: &EntityId, session_id: Option<u32>) -> Result<(), CallError> {
        let params = session_id.map(|id| json!({"session_id": id}));
        let command = ExecuteEntityCommand { entity_id: entity_id.clone(), cmd_id: messages::VOICE_END, params };
        self.call(&command).await.map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_serialize_as_the_spec_shows() {
        assert_eq!(serde_json::to_value(GetVoiceControlCfg {}).unwrap(), json!({}));
        assert_eq!(serde_json::to_value(GetVoiceAssistants {}).unwrap(), json!({}));
        let update = VoiceControlUpdate {
            microphone: Some(true),
            voice_assistant: Some(VoiceAssistantUpdate { entity_id: "r3cui.voice".into(), profile_id: Some("p1".into()), speech_response: Some(false) }),
        };
        assert_eq!(
            serde_json::to_value(update).unwrap(),
            json!({"microphone": true, "voice_assistant": {"entity_id": "r3cui.voice", "profile_id": "p1", "speech_response": false}})
        );
        let only_mic = VoiceControlUpdate { microphone: Some(false), voice_assistant: None };
        assert_eq!(serde_json::to_value(only_mic).unwrap(), json!({"microphone": false}));
    }

    #[test]
    fn voice_commands_serialize_as_execute_entity_command() {
        let id = EntityId::new("r3cui.voice");
        let start = VoiceStart { profile_id: Some("p1".into()), speech_response: true, ..VoiceStart::new(7) };
        let command = ExecuteEntityCommand { entity_id: id.clone(), cmd_id: messages::VOICE_START, params: Some(serde_json::to_value(&start).unwrap()) };
        assert_eq!(
            serde_json::to_value(command).unwrap(),
            json!({"entity_id": "r3cui.voice", "cmd_id": "voice_start", "params": {"session_id": 7, "timeout": 15, "speech_response": true, "profile_id": "p1"}})
        );
        let end = ExecuteEntityCommand { entity_id: id, cmd_id: messages::VOICE_END, params: None };
        assert_eq!(serde_json::to_value(end).unwrap(), json!({"entity_id": "r3cui.voice", "cmd_id": "voice_end"}));
    }

    #[test]
    fn audio_cfg_uses_the_spec_names() {
        let start = VoiceStart { audio_cfg: Some(AudioCfg { channels: 1, sample_rate: 8000, sample_format: "I16" }), ..VoiceStart::new(8) };
        assert_eq!(serde_json::to_value(start).unwrap()["audio_cfg"], json!({"channels": 1, "sample_rate": 8000, "sample_format": "I16"}));
    }

    #[test]
    fn the_config_and_assistants_parse() {
        let cfg: VoiceControlCfg = serde_json::from_value(json!({
            "microphone": true,
            "voice_assistant": {
                "active": {
                    "entity_id": "r3cui.voice", "name": {"en": "Assist"}, "state": "ON", "features": ["transcription", "response_text", "future"],
                    "profiles": [{"id": "p1", "name": "English", "language": "en", "features": []}], "preferred_profile": "p1"
                },
                "profile_id": "p1", "speech_response": true
            }
        }))
        .unwrap();
        let va = cfg.voice_assistant.active.unwrap();
        assert_eq!((va.entity_id.0.as_str(), va.features[2], va.profiles[0].features.as_deref()), ("r3cui.voice", VoiceFeature::Unknown, Some(&[][..])));
        assert!(cfg.microphone && cfg.voice_assistant.speech_response);

        let none: VoiceControlCfg = serde_json::from_value(json!({"microphone": false, "voice_assistant": {}})).unwrap();
        assert!(none.voice_assistant.active.is_none() && !none.voice_assistant.speech_response);
    }
}
