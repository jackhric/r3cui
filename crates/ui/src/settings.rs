//! User settings, read once at launch from `settings.json` in `UC_CONFIG_HOME` (the archive's
//! `config/` directory on the remote) or `./config` on the desktop.

use serde_json::Value;
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug, Default)]
pub struct Settings {
    /// The interface colour when the current input has none of its own.
    pub accent: Option<String>,
    /// Input name fragment (lower case) → colour. The first fragment found in the input's name wins.
    pub source_accents: Vec<(String, String)>,
    /// Device only: "femtovg" (GLES2, the default) or "software".
    #[cfg_attr(not(feature = "device"), allow(dead_code))]
    pub renderer: Option<String>,
    /// Environment variables to set before the display starts, for diagnostics such as
    /// `SLINT_DEBUG_PERFORMANCE`.
    pub env: Vec<(String, String)>,
    /// Entity-id endings of the Home Assistant helpers that steer the key backlight.
    pub keys_light: String,
    pub keys_mode: String,
    /// Receiver inputs to leave out of the input list, compared without regard to case.
    pub hidden_sources: Vec<String>,
    /// How strongly the menu's lava background shows, 0 (off) to 1.
    pub lava: Option<f32>,
    /// The least time between two volume commands from the slider, so the receiver keeps up.
    pub volume_interval_ms: Option<u64>,
    /// Entity-id ending of the Home Assistant sensor holding the receiver's volume limit.
    pub volume_limit_sensor: String,
    /// Haptic feedback on or off; the core's own setting must also allow it.
    pub haptics_enabled: Option<bool>,
    /// 0 (off) to 1 (full): lowers the strength of every pattern.
    pub haptics_intensity: Option<f32>,
    pub sound_enabled: Option<bool>,
    /// 0 to 1, multiplied by the core's sound volume.
    pub sound_volume: Option<f32>,
    /// How the poster wall shows the highlighted title: 1 sheet, 2 overlay, 3 inline.
    pub wall_style: Option<i32>,
}

impl Settings {
    /// The colour for an input. With no receiver there is no input, and the screen is Kodi's.
    pub fn accent_for(&self, source: &str) -> Option<&str> {
        let name = if source.trim().is_empty() { "kodi".to_string() } else { source.to_lowercase() };
        self.source_accents
            .iter()
            .find(|(fragment, _)| name.contains(fragment.as_str()))
            .map(|(_, colour)| colour.as_str())
            .or(self.accent.as_deref())
    }

    pub fn hides(&self, source: &str) -> bool {
        self.hidden_sources.iter().any(|h| h.eq_ignore_ascii_case(source.trim()))
    }
}

pub fn get() -> &'static Settings {
    static SETTINGS: OnceLock<Settings> = OnceLock::new();
    SETTINGS.get_or_init(|| {
        let dir = std::env::var_os("UC_CONFIG_HOME").map_or_else(|| PathBuf::from("config"), PathBuf::from);
        let path = dir.join("settings.json");
        match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| parse(&t)) {
            Ok(settings) => {
                tracing::info!(?settings, "settings from {}", path.display());
                settings
            }
            Err(e) => {
                tracing::warn!("no usable {}: {e}", path.display());
                Settings::default()
            }
        }
    })
}

fn string_or(v: &Value, key: &str, default: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or(default).to_string()
}

fn parse(text: &str) -> Result<Settings, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    Ok(Settings {
        accent: v.get("accent").and_then(Value::as_str).map(String::from),
        source_accents: v
            .get("source_accents")
            .and_then(Value::as_object)
            .map(|m| m.iter().filter_map(|(k, c)| Some((k.to_lowercase(), c.as_str()?.to_string()))).collect())
            .unwrap_or_default(),
        renderer: v.get("renderer").and_then(Value::as_str).map(String::from),
        env: v
            .get("env")
            .and_then(Value::as_object)
            .map(|m| m.iter().filter_map(|(k, x)| Some((k.clone(), x.as_str()?.to_string()))).collect())
            .unwrap_or_default(),
        keys_light: string_or(&v, "keys_light", "light.theater_remote_keys"),
        keys_mode: string_or(&v, "keys_mode", "theater_remote_keys_mode"),
        hidden_sources: v
            .get("hidden_sources")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
            .unwrap_or_default(),
        volume_limit_sensor: string_or(&v, "volume_limit_sensor", "sensor.theater_receiver_volume_limit"),
        volume_interval_ms: v.get("volume_interval_ms").and_then(Value::as_u64),
        lava: v.get("lava").and_then(Value::as_f64).map(|x| x.clamp(0.0, 1.0) as f32),
        haptics_enabled: v.get("haptics_enabled").and_then(Value::as_bool),
        haptics_intensity: v.get("haptics_intensity").and_then(Value::as_f64).map(|x| x.clamp(0.0, 1.0) as f32),
        sound_enabled: v.get("sound_enabled").and_then(Value::as_bool),
        sound_volume: v.get("sound_volume").and_then(Value::as_f64).map(|x| x.clamp(0.0, 1.0) as f32),
        wall_style: v.get("wall_style").and_then(Value::as_i64).map(|x| x.clamp(1, 3) as i32),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_sources_ignore_case() {
        let s = parse(r#"{"hidden_sources": ["NET", "HEOS Music"]}"#).unwrap();
        assert!(s.hides("heos music") && s.hides("Net"));
        assert!(!s.hides("KODI"));
        assert_eq!(s.accent, None);
    }

    #[test]
    fn feedback_settings_are_clamped() {
        let s = parse(r#"{"haptics_enabled": false, "haptics_intensity": 3, "sound_volume": 0.25}"#).unwrap();
        assert_eq!((s.haptics_enabled, s.haptics_intensity, s.sound_volume), (Some(false), Some(1.0), Some(0.25)));
        assert_eq!(s.sound_enabled, None);
    }

    #[test]
    fn inputs_pick_their_colour() {
        let s = parse(r##"{"accent": "#FFFFFF", "source_accents": {"kodi": "#FF0000", "PlayStation": "#0000FF"}}"##).unwrap();
        assert_eq!(s.accent_for("PLAYSTATION 5"), Some("#0000FF"));
        assert_eq!(s.accent_for("KODI"), Some("#FF0000"));
        assert_eq!(s.accent_for(""), Some("#FF0000"), "no receiver means the Kodi screen");
        assert_eq!(s.accent_for("TV Audio"), Some("#FFFFFF"));
    }
}
