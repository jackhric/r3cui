//! User settings, read once at launch from `settings.json` in `UC_CONFIG_HOME` (the archive's
//! `config/` directory on the remote) or `./config` on the desktop.

use serde::Deserialize;
use std::collections::BTreeMap;
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
    /// How the poster wall shows the highlighted title: 1 sheet, 2 overlay (the default), 3 inline.
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

/// `settings.json` as written. Every field is optional; unknown ones are ignored.
#[derive(Deserialize, Default)]
#[serde(default)]
struct File {
    accent: Option<String>,
    source_accents: BTreeMap<String, String>,
    renderer: Option<String>,
    env: BTreeMap<String, String>,
    keys_light: Option<String>,
    keys_mode: Option<String>,
    hidden_sources: Vec<String>,
    lava: Option<f32>,
    volume_interval_ms: Option<u64>,
    volume_limit_sensor: Option<String>,
    haptics_enabled: Option<bool>,
    haptics_intensity: Option<f32>,
    sound_enabled: Option<bool>,
    sound_volume: Option<f32>,
    wall_style: Option<i32>,
}

fn parse(text: &str) -> Result<Settings, String> {
    let f: File = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let unit = |x: Option<f32>| x.map(|x| x.clamp(0.0, 1.0));
    Ok(Settings {
        accent: f.accent,
        source_accents: f.source_accents.into_iter().map(|(k, c)| (k.to_lowercase(), c)).collect(),
        renderer: f.renderer,
        env: f.env.into_iter().collect(),
        keys_light: f.keys_light.unwrap_or_else(|| "light.theater_remote_keys".into()),
        keys_mode: f.keys_mode.unwrap_or_else(|| "theater_remote_keys_mode".into()),
        hidden_sources: f.hidden_sources,
        volume_limit_sensor: f.volume_limit_sensor.unwrap_or_else(|| "sensor.theater_receiver_volume_limit".into()),
        volume_interval_ms: f.volume_interval_ms,
        lava: unit(f.lava),
        haptics_enabled: f.haptics_enabled,
        haptics_intensity: unit(f.haptics_intensity),
        sound_enabled: f.sound_enabled,
        sound_volume: unit(f.sound_volume),
        wall_style: f.wall_style.map(|x| x.clamp(1, 3)),
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
    fn defaults_fill_what_is_missing() {
        let s = parse("{}").unwrap();
        assert_eq!(s.keys_light, "light.theater_remote_keys");
        assert_eq!(s.volume_limit_sensor, "sensor.theater_receiver_volume_limit");
        assert_eq!(parse(r#"{"wall_style": 9}"#).unwrap().wall_style, Some(3));
        let e = parse(r#"{"lava": "high"}"#).unwrap_err();
        assert!(e.contains("line 1 column"), "{e}");
    }

    #[test]
    fn the_shipped_file_parses() {
        let s = parse(include_str!("../../../../config/settings.json")).unwrap();
        assert!(s.hides("NET") && s.accent_for("PlayStation 5").is_some());
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
