//! Home Assistant's WebSocket vocabulary as it appears on the wire: message types, the frame fields the client and
//! the fake server both touch, and the domains, services, states and attributes the remote uses.

/// The `type` of a frame, either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageType {
    AuthRequired,
    Auth,
    AuthOk,
    AuthInvalid,
    Ping,
    Pong,
    GetStates,
    SubscribeEntities,
    UnsubscribeEvents,
    CallService,
    AssistPipelineList,
    AssistPipelineRun,
    Result,
    Event,
}

impl MessageType {
    const ALL: [Self; 14] = [
        Self::AuthRequired,
        Self::Auth,
        Self::AuthOk,
        Self::AuthInvalid,
        Self::Ping,
        Self::Pong,
        Self::GetStates,
        Self::SubscribeEntities,
        Self::UnsubscribeEvents,
        Self::CallService,
        Self::AssistPipelineList,
        Self::AssistPipelineRun,
        Self::Result,
        Self::Event,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AuthRequired => "auth_required",
            Self::Auth => "auth",
            Self::AuthOk => "auth_ok",
            Self::AuthInvalid => "auth_invalid",
            Self::Ping => "ping",
            Self::Pong => "pong",
            Self::GetStates => "get_states",
            Self::SubscribeEntities => "subscribe_entities",
            Self::UnsubscribeEvents => "unsubscribe_events",
            Self::CallService => "call_service",
            Self::AssistPipelineList => "assist_pipeline/pipeline/list",
            Self::AssistPipelineRun => "assist_pipeline/run",
            Self::Result => "result",
            Self::Event => "event",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == text)
    }
}

/// Keys in frames.
pub mod field {
    pub const ID: &str = "id";
    pub const TYPE: &str = "type";
    pub const ACCESS_TOKEN: &str = "access_token";
    pub const HA_VERSION: &str = "ha_version";
    pub const MESSAGE: &str = "message";
    pub const SUCCESS: &str = "success";
    pub const RESULT: &str = "result";
    pub const ERROR: &str = "error";
    pub const CODE: &str = "code";
    pub const EVENT: &str = "event";
    pub const ENTITY_ID: &str = "entity_id";
    pub const ENTITY_IDS: &str = "entity_ids";
    pub const SUBSCRIPTION: &str = "subscription";
    pub const DOMAIN: &str = "domain";
    pub const SERVICE: &str = "service";
    pub const SERVICE_DATA: &str = "service_data";
    pub const TARGET: &str = "target";
    pub const STATE: &str = "state";
    pub const ATTRIBUTES: &str = "attributes";
}

/// Assist: the voice pipelines, and the events a run reports.
pub mod assist {
    /// Keys in the pipeline list, the run request and the run's events.
    pub mod field {
        pub const PIPELINES: &str = "pipelines";
        pub const PREFERRED_PIPELINE: &str = "preferred_pipeline";
        pub const PIPELINE: &str = "pipeline";
        pub const NAME: &str = "name";
        pub const LANGUAGE: &str = "language";
        pub const START_STAGE: &str = "start_stage";
        pub const END_STAGE: &str = "end_stage";
        pub const INPUT: &str = "input";
        pub const SAMPLE_RATE: &str = "sample_rate";
        pub const TIMEOUT: &str = "timeout";
        pub const DATA: &str = "data";
        pub const RUNNER_DATA: &str = "runner_data";
        pub const STT_BINARY_HANDLER_ID: &str = "stt_binary_handler_id";
        pub const STT_OUTPUT: &str = "stt_output";
        pub const TEXT: &str = "text";
        pub const INTENT_OUTPUT: &str = "intent_output";
        pub const RESPONSE: &str = "response";
        pub const RESPONSE_TYPE: &str = "response_type";
        pub const SPEECH: &str = "speech";
        pub const PLAIN: &str = "plain";
        pub const TTS_OUTPUT: &str = "tts_output";
        pub const URL: &str = "url";
        pub const MIME_TYPE: &str = "mime_type";
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Stage {
        Stt,
        Intent,
        Tts,
    }

    impl Stage {
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::Stt => "stt",
                Self::Intent => "intent",
                Self::Tts => "tts",
            }
        }
    }

    /// The `type` of an event of a run.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum EventType {
        RunStart,
        RunEnd,
        WakeWordStart,
        WakeWordEnd,
        SttStart,
        SttVadStart,
        SttVadEnd,
        SttEnd,
        IntentStart,
        IntentProgress,
        IntentEnd,
        TtsStart,
        TtsEnd,
        Error,
    }

    impl EventType {
        pub const ALL: [Self; 14] = [
            Self::RunStart,
            Self::RunEnd,
            Self::WakeWordStart,
            Self::WakeWordEnd,
            Self::SttStart,
            Self::SttVadStart,
            Self::SttVadEnd,
            Self::SttEnd,
            Self::IntentStart,
            Self::IntentProgress,
            Self::IntentEnd,
            Self::TtsStart,
            Self::TtsEnd,
            Self::Error,
        ];

        pub const fn as_str(self) -> &'static str {
            match self {
                Self::RunStart => "run-start",
                Self::RunEnd => "run-end",
                Self::WakeWordStart => "wake_word-start",
                Self::WakeWordEnd => "wake_word-end",
                Self::SttStart => "stt-start",
                Self::SttVadStart => "stt-vad-start",
                Self::SttVadEnd => "stt-vad-end",
                Self::SttEnd => "stt-end",
                Self::IntentStart => "intent-start",
                Self::IntentProgress => "intent-progress",
                Self::IntentEnd => "intent-end",
                Self::TtsStart => "tts-start",
                Self::TtsEnd => "tts-end",
                Self::Error => "error",
            }
        }

        pub fn parse(text: &str) -> Option<Self> {
            Self::ALL.into_iter().find(|t| t.as_str() == text)
        }
    }

    /// `intent_output.response.response_type` of an answer that worked.
    pub mod response_type {
        pub const ACTION_DONE: &str = "action_done";
        pub const QUERY_ANSWER: &str = "query_answer";
    }

    /// The `code` of an error event.
    pub mod error_code {
        pub const TIMEOUT: &str = "timeout";
        pub const NO_TEXT_RECOGNIZED: &str = "stt-no-text-recognized";
        pub const UNSUPPORTED_AUDIO: &str = "stt-provider-unsupported-metadata";
        pub const WAKE_ENGINE_MISSING: &str = "wake-engine-missing";
        pub const WAKE_PROVIDER_MISSING: &str = "wake-provider-missing";
        pub const WAKE_STREAM_FAILED: &str = "wake-stream-failed";
        pub const WAKE_WORD_TIMEOUT: &str = "wake-word-timeout";
        pub const STT_PROVIDER_MISSING: &str = "stt-provider-missing";
        pub const INTENT_NOT_SUPPORTED: &str = "intent-not-supported";
        pub const TTS_NOT_SUPPORTED: &str = "tts-not-supported";
    }
}

pub mod domain {
    pub const MEDIA_PLAYER: &str = "media_player";
    pub const LIGHT: &str = "light";
    pub const SELECT: &str = "select";
    pub const INPUT_SELECT: &str = "input_select";
    pub const SENSOR: &str = "sensor";
    pub const INPUT_NUMBER: &str = "input_number";
}

/// Services of the `media_player` domain.
pub mod service {
    pub const VOLUME_SET: &str = "volume_set";
    pub const VOLUME_UP: &str = "volume_up";
    pub const VOLUME_DOWN: &str = "volume_down";
    pub const VOLUME_MUTE: &str = "volume_mute";
    pub const SELECT_SOURCE: &str = "select_source";
    pub const TURN_ON: &str = "turn_on";
    pub const TURN_OFF: &str = "turn_off";
}

pub mod state {
    pub const ON: &str = "on";
    pub const OFF: &str = "off";
    pub const STANDBY: &str = "standby";
    pub const UNAVAILABLE: &str = "unavailable";
    pub const UNKNOWN: &str = "unknown";
}

/// Attributes of entities, and the service-data keys that set the same things.
pub mod attribute {
    pub const FRIENDLY_NAME: &str = "friendly_name";
    pub const VOLUME_LEVEL: &str = "volume_level";
    pub const IS_VOLUME_MUTED: &str = "is_volume_muted";
    pub const SOURCE: &str = "source";
    pub const SOURCE_LIST: &str = "source_list";
    pub const SUPPORTED_FEATURES: &str = "supported_features";
    pub const BRIGHTNESS: &str = "brightness";
    pub const HS_COLOR: &str = "hs_color";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_type_parses_back_from_its_name() {
        for t in MessageType::ALL {
            assert_eq!(MessageType::parse(t.as_str()), Some(t));
        }
        assert_eq!(MessageType::parse("nope"), None);
    }

    #[test]
    fn every_event_type_parses_back_from_its_name() {
        for t in assist::EventType::ALL {
            assert_eq!(assist::EventType::parse(t.as_str()), Some(t));
        }
    }
}
