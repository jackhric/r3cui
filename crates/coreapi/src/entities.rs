//! The core's configured entities (`get_entities`, `Entity` in the AsyncAPI spec), and the media-player attributes the
//! UI reads. Attributes stay a map, since each entity type has its own set and `entity_change` sends only what changed,
//! but they are read by name through `Attr`, never by string.

use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    Button,
    Climate,
    Cover,
    Light,
    MediaPlayer,
    Sensor,
    Switch,
    Activity,
    Macro,
    Remote,
    IrEmitter,
    Select,
    Number,
    VoiceAssistant,
    #[serde(other)]
    Unknown,
}

/// A media player's state, as the core's entity docs list them. Anything else, or none, is `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MediaState {
    On,
    Off,
    Playing,
    Paused,
    Standby,
    Buffering,
    Unavailable,
    #[serde(other)]
    Unknown,
}

impl MediaState {
    /// Something is loaded and on its way or showing.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Playing | Self::Paused | Self::Buffering)
    }

    /// The device can't be reached or hasn't said.
    pub fn is_gone(self) -> bool {
        matches!(self, Self::Unavailable | Self::Unknown)
    }
}

/// The attributes the UI reads, by their names in the core's entity docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attr {
    State,
    Volume,
    Muted,
    Source,
    SourceList,
    SoundMode,
    SoundModeList,
    MediaTitle,
    MediaArtist,
    MediaType,
    MediaDuration,
    MediaPosition,
    MediaImageUrl,
    /// A sensor's or select's value.
    Value,
    CurrentOption,
    Hue,
    Saturation,
    Brightness,
}

impl Attr {
    pub fn name(self) -> &'static str {
        match self {
            Self::State => "state",
            Self::Volume => "volume",
            Self::Muted => "muted",
            Self::Source => "source",
            Self::SourceList => "source_list",
            Self::SoundMode => "sound_mode",
            Self::SoundModeList => "sound_mode_list",
            Self::MediaTitle => "media_title",
            Self::MediaArtist => "media_artist",
            Self::MediaType => "media_type",
            Self::MediaDuration => "media_duration",
            Self::MediaPosition => "media_position",
            Self::MediaImageUrl => "media_image_url",
            Self::Value => "value",
            Self::CurrentOption => "current_option",
            Self::Hue => "hue",
            Self::Saturation => "saturation",
            Self::Brightness => "brightness",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Entity {
    pub entity_id: String,
    pub entity_type: EntityType,
    #[serde(default)]
    pub integration_id: Option<String>,
    #[serde(default, deserialize_with = "language_text")]
    pub name: Option<String>,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub attributes: Map<String, Value>,
}

impl Entity {
    /// Its name in English, or any language it has, or its id.
    pub fn name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.entity_id)
    }

    pub fn has_feature(&self, feature: &str) -> bool {
        self.features.iter().any(|f| f == feature)
    }

    pub fn text(&self, a: Attr) -> &str {
        self.attributes.get(a.name()).and_then(Value::as_str).unwrap_or_default()
    }

    pub fn number(&self, a: Attr) -> Option<f64> {
        self.attributes.get(a.name()).and_then(Value::as_f64)
    }

    pub fn flag(&self, a: Attr) -> Option<bool> {
        self.attributes.get(a.name()).and_then(Value::as_bool)
    }

    pub fn list(&self, a: Attr) -> Vec<String> {
        self.attributes.get(a.name()).and_then(Value::as_array).map_or_else(Vec::new, |l| {
            l.iter().filter_map(Value::as_str).map(String::from).collect()
        })
    }

    pub fn state(&self) -> MediaState {
        self.attributes.get(Attr::State.name()).and_then(|s| MediaState::deserialize(s).ok()).unwrap_or(MediaState::Unknown)
    }

    /// Merge an `entity_change`'s attributes into these.
    pub fn apply(&mut self, changed: &Map<String, Value>) {
        for (k, v) in changed {
            self.attributes.insert(k.clone(), v.clone());
        }
    }
}

/// `name` comes as a `LanguageText` map (`{"en": "…"}`), or from older cores as a plain string.
fn language_text<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match Option::<Value>::deserialize(d)? {
        Some(Value::String(s)) => Some(s),
        Some(Value::Object(m)) => m.get("en").or_else(|| m.values().next()).and_then(Value::as_str).map(String::from),
        _ => None,
    })
}

/// Entities that don't match the model are left out (with a log line) rather than failing the whole list.
pub(crate) fn parse_all(items: Vec<Value>) -> Vec<Entity> {
    items
        .into_iter()
        .filter_map(|v| match serde_json::from_value::<Entity>(v.clone()) {
            Ok(e) => Some(e),
            Err(err) => {
                let id = v.get("entity_id").and_then(Value::as_str).unwrap_or("?").to_string();
                tracing::warn!("skipping entity {id}: {err}");
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn kodi() -> Entity {
        serde_json::from_value(json!({
            "entity_id": "kodi_driver.main.media_player.kodi.lan",
            "entity_type": "media_player",
            "integration_id": "kodi_driver.main",
            "name": { "en": "Kodi", "de": "Kodi" },
            "features": ["on_off", "volume_up_down", "mute_toggle"],
            "attributes": { "state": "PLAYING", "media_title": "Moon", "media_duration": 5820, "sound_mode_list": ["English DTS"] }
        }))
        .unwrap()
    }

    #[test]
    fn reads_a_media_player() {
        let e = kodi();
        assert_eq!((e.entity_type, e.name(), e.state()), (EntityType::MediaPlayer, "Kodi", MediaState::Playing));
        assert!(e.has_feature("mute_toggle") && !e.has_feature("select_source"));
        assert_eq!((e.text(Attr::MediaTitle), e.number(Attr::MediaDuration)), ("Moon", Some(5820.0)));
        assert_eq!(e.list(Attr::SoundModeList), ["English DTS"]);
        assert_eq!((e.text(Attr::Source), e.flag(Attr::Muted)), ("", None));
    }

    #[test]
    fn changes_merge_and_unknowns_are_tolerated() {
        let mut e = kodi();
        e.apply(json!({ "state": "LOADING", "media_title": "Alien" }).as_object().unwrap());
        assert_eq!((e.state(), e.text(Attr::MediaTitle)), (MediaState::Unknown, "Alien"));
        let plain: Entity = serde_json::from_value(json!({ "entity_id": "x.y", "entity_type": "toaster", "name": "Old" })).unwrap();
        assert_eq!((plain.entity_type, plain.name()), (EntityType::Unknown, "Old"));
        assert_eq!(parse_all(vec![json!({ "entity_id": "no.type" }), json!({ "entity_id": "a.b", "entity_type": "light" })]).len(), 1);
    }
}
