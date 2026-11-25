//! The health of each link the remote depends on, for the status page and the home screen's warning.

use coreapi::MediaState;
use serde_json::Value;
use std::collections::HashMap;



#[derive(Clone, Debug, PartialEq)]
pub struct StatusLine {
    pub label: &'static str,
    pub value: String,
    /// 0 working, 1 broken, 2 unknown because something it depends on is broken
    pub level: i32,
}

/// What we know about each link. `None` means that link is not part of this setup.
pub struct Health<'a> {
    pub wifi_ssid: &'a str,
    pub home_assistant: Option<bool>,
    pub receiver: Option<MediaState>,
    pub kodi_link: Option<bool>,
    pub kodi: Option<MediaState>,
    pub on_kodi: bool,
}

/// The status page's rows, and the one-line warning for the home screen (empty when all is well).
pub fn health(h: Health) -> (Vec<StatusLine>, String) {
    let line = |label, value: &str, level| StatusLine { label, value: value.to_string(), level };
    let link = |label, up: bool| line(label, if up { "Connected" } else { "Unreachable" }, if up { 0 } else { 1 });

    let mut rows = vec![if h.wifi_ssid.is_empty() { line("Wi-Fi", "Disconnected", 1) } else { line("Wi-Fi", h.wifi_ssid, 0) }];
    if let Some(up) = h.home_assistant {
        rows.push(link("Home Assistant", up));
    }
    if let Some(state) = h.receiver {
        rows.push(if h.home_assistant == Some(false) {
            line("Receiver", "Unknown", 2)
        } else if state.is_gone() {
            line("Receiver", "Unreachable", 1)
        } else {
            line("Receiver", if state == MediaState::Off { "Off" } else { "On" }, 0)
        });
    }
    if let Some(up) = h.kodi_link {
        rows.push(link("Kodi link", up));
    }
    if let Some(state) = h.kodi {
        rows.push(if h.kodi_link == Some(false) {
            line("Kodi", "Unknown", 2)
        } else if state.is_gone() {
            line("Kodi", "Unreachable", 1)
        } else {
            let word = match state {
                MediaState::Playing => "Playing",
                MediaState::Paused => "Paused",
                MediaState::Off => "Off",
                _ => "Idle",
            };
            line("Kodi", word, 0)
        });
    }

    // The first broken link in the chain is the cause; the rest are consequences.
    let warning = rows
        .iter()
        .find(|r| r.level == 1 && (h.on_kodi || !r.label.starts_with("Kodi")))
        .map(|r| match r.label {
            "Wi-Fi" => "Wi-Fi disconnected".to_string(),
            label => format!("{label} unreachable"),
        })
        .unwrap_or_default();
    (rows, warning)
}




/// integration id → whether the core is connected to the thing behind it.
pub async fn integrations(core: &coreapi::Core) -> HashMap<String, bool> {
    core.fetch("get_integration_status")
        .await
        .and_then(|d| d.get("status").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|s| {
            let id = s.get("integration_id")?.as_str()?.to_string();
            Some((id, s.get("device_state").and_then(Value::as_str) == Some("CONNECTED")))
        })
        .collect()
}


#[cfg(test)]
mod tests {
    use super::*;

    fn all_up() -> Health<'static> {
        Health { wifi_ssid: "home", home_assistant: Some(true), receiver: Some(MediaState::On), kodi_link: Some(true), kodi: Some(MediaState::Playing), on_kodi: true }
    }

    #[test]
    fn healthy_has_no_warning() {
        let (rows, warning) = health(all_up());
        assert!(rows.iter().all(|r| r.level == 0));
        assert_eq!(warning, "");
        assert_eq!(rows[4].value, "Playing");
    }

    #[test]
    fn a_broken_link_greys_what_depends_on_it() {
        let (rows, warning) = health(Health { home_assistant: Some(false), ..all_up() });
        assert_eq!((rows[1].level, rows[2].level, rows[2].value.as_str()), (1, 2, "Unknown"));
        assert_eq!(warning, "Home Assistant unreachable");
    }

    #[test]
    fn kodi_trouble_only_matters_on_the_kodi_screen() {
        let down = || Health { kodi: Some(MediaState::Unavailable), ..all_up() };
        assert_eq!(health(down()).1, "Kodi unreachable");
        assert_eq!(health(Health { on_kodi: false, ..down() }).1, "");
    }
}

