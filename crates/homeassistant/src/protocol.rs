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
    Result,
    Event,
}

impl MessageType {
    const ALL: [Self; 12] = [
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

pub mod domain {
    pub const MEDIA_PLAYER: &str = "media_player";
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
}
