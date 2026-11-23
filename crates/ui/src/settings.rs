//! User settings, read once at launch from `settings.json` in `UC_CONFIG_HOME` (the archive's
//! `config/` directory on the remote) or `./config` on the desktop.

use serde_json::Value;
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug, Default)]
pub struct Settings {
    pub accent: Option<String>,
    /// Receiver inputs to leave out of the input list, compared without regard to case.
    pub hidden_sources: Vec<String>,
}

impl Settings {
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

fn parse(text: &str) -> Result<Settings, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    Ok(Settings {
        accent: v.get("accent").and_then(Value::as_str).map(String::from),
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
}
