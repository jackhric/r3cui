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
    /// Entity-id endings of the Home Assistant helpers that steer the key backlight.
    pub keys_light: String,
    pub keys_mode: String,
    /// Receiver inputs to leave out of the input list, compared without regard to case.
    pub hidden_sources: Vec<String>,
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
        keys_light: string_or(&v, "keys_light", "light.theater_remote_keys"),
        keys_mode: string_or(&v, "keys_mode", "theater_remote_keys_mode"),
        hidden_sources: v
            .get("hidden_sources")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
            .unwrap_or_default(),
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
    fn inputs_pick_their_colour() {
        let s = parse(r##"{"accent": "#FFFFFF", "source_accents": {"kodi": "#FF0000", "PlayStation": "#0000FF"}}"##).unwrap();
        assert_eq!(s.accent_for("PLAYSTATION 5"), Some("#0000FF"));
        assert_eq!(s.accent_for("KODI"), Some("#FF0000"));
        assert_eq!(s.accent_for(""), Some("#FF0000"), "no receiver means the Kodi screen");
        assert_eq!(s.accent_for("TV Audio"), Some("#FFFFFF"));
    }
}
