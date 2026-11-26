//! Typed reads over an `EntityState`: what a media player or a light's attributes mean.

use crate::protocol::{attribute, state};
use crate::state::EntityState;

/// Home Assistant's `MediaPlayerEntityFeature` bits, as listed in `supported_features`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {
    VolumeSet = 4,
    VolumeMute = 8,
    TurnOn = 128,
    TurnOff = 256,
    VolumeStep = 1024,
    SelectSource = 2048,
}

#[derive(Clone, Copy, Debug)]
pub struct MediaPlayer<'a>(pub &'a EntityState);

impl MediaPlayer<'_> {
    pub fn is_on(&self) -> bool {
        !matches!(self.0.state.as_str(), state::OFF | state::STANDBY | state::UNAVAILABLE | state::UNKNOWN)
    }

    /// 0 to 1.
    pub fn volume_level(&self) -> Option<f64> {
        self.0.attr_f64(attribute::VOLUME_LEVEL)
    }

    pub fn muted(&self) -> Option<bool> {
        self.0.attr_bool(attribute::IS_VOLUME_MUTED)
    }

    pub fn source(&self) -> Option<&str> {
        self.0.attr_str(attribute::SOURCE)
    }

    pub fn source_list(&self) -> Vec<&str> {
        let list = self.0.attr(attribute::SOURCE_LIST).and_then(|v| v.as_array());
        list.map(|a| a.iter().filter_map(|s| s.as_str()).collect()).unwrap_or_default()
    }

    pub fn supported(&self, feature: Feature) -> bool {
        let bits = self.0.attr(attribute::SUPPORTED_FEATURES).and_then(|v| v.as_u64()).unwrap_or(0);
        bits & feature as u64 != 0
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Light<'a>(pub &'a EntityState);

impl Light<'_> {
    pub fn is_on(&self) -> bool {
        self.0.state == state::ON
    }

    /// 0 to 255.
    pub fn brightness(&self) -> Option<u8> {
        let b = self.0.attr_f64(attribute::BRIGHTNESS)?;
        Some(b.clamp(0.0, 255.0).round() as u8)
    }

    /// Hue in degrees, then saturation 0 to 100.
    pub fn hs_color(&self) -> Option<(f64, f64)> {
        let pair = self.0.attr(attribute::HS_COLOR)?.as_array()?;
        Some((pair.first()?.as_f64()?, pair.get(1)?.as_f64()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entity(state: &str, attrs: serde_json::Value) -> EntityState {
        EntityState::new(state, attrs.as_object().unwrap().clone())
    }

    #[test]
    fn the_theater_receiver() {
        let e = entity(
            "on",
            json!({
                "source_list": ["ANDROID TV", "Bluetooth", "KODI"], "volume_level": 0.56, "is_volume_muted": false,
                "source": "KODI", "device_class": "receiver", "friendly_name": "Theater Receiver", "supported_features": 69004
            }),
        );
        let p = MediaPlayer(&e);
        assert!(p.is_on());
        assert_eq!((p.volume_level(), p.muted(), p.source()), (Some(0.56), Some(false), Some("KODI")));
        assert_eq!(p.source_list(), ["ANDROID TV", "Bluetooth", "KODI"]);
        for f in [Feature::VolumeSet, Feature::VolumeMute, Feature::TurnOn, Feature::TurnOff, Feature::VolumeStep, Feature::SelectSource] {
            assert!(p.supported(f), "{f:?}");
        }
        assert!(!MediaPlayer(&entity("on", json!({"supported_features": 4}))).supported(Feature::TurnOff));
    }

    #[test]
    fn off_states_are_not_on() {
        for s in ["off", "standby", "unavailable", "unknown"] {
            assert!(!MediaPlayer(&entity(s, json!({}))).is_on(), "{s}");
        }
        assert!(MediaPlayer(&entity("playing", json!({}))).is_on());
    }

    #[test]
    fn a_light() {
        let e = entity("on", json!({"brightness": 128, "hs_color": [30.5, 80.0]}));
        let l = Light(&e);
        assert!(l.is_on());
        assert_eq!((l.brightness(), l.hs_color()), (Some(128), Some((30.5, 80.0))));
        let off = entity("off", json!({"brightness": null, "hs_color": null}));
        assert_eq!((Light(&off).is_on(), Light(&off).brightness(), Light(&off).hs_color()), (false, None, None));
    }
}
