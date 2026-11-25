//! Wi-Fi networks and the health of each link the remote depends on.

use coreapi::MediaState;
use serde_json::{json, Value};
use std::collections::HashMap;

/// How many networks fit on the Wi-Fi page above "Scan again".
const MAX_NETWORKS: usize = 5;

#[derive(Clone, Debug, PartialEq)]
pub struct Network {
    pub ssid: String,
    /// The SSID's raw bytes, hex encoded: what the core wants when joining from a scan result.
    pub hex: String,
    pub secured: bool,
    /// 1 to 4
    pub bars: i32,
    pub connected: bool,
}

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

fn bars(dbm: i64) -> i32 {
    match dbm {
        d if d >= -55 => 4,
        d if d >= -67 => 3,
        d if d >= -78 => 2,
        _ => 1,
    }
}


/// The connected network's name (empty when disconnected) and what the last scan found.
pub async fn wifi(core: &coreapi::Core) -> (String, Vec<Network>) {
    // The simulator has no Wi-Fi; previews of it can name a network to show as joined.
    #[cfg(feature = "preview")]
    if let Ok(ssid) = std::env::var("UC_PREVIEW_WIFI") {
        return (ssid.clone(), demo_networks(&ssid));
    }
    let (status, scan) = tokio::join!(core.fetch("get_wifi_status"), core.fetch("get_wifi_scan_status"));
    let field = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let connected = status.as_ref().filter(|s| field(s, "wpa_state") == "COMPLETED");
    let ssid = connected.map(|s| field(s, "ssid")).unwrap_or_default();

    let mut best: HashMap<String, (i64, Network)> = HashMap::new();
    let mut consider = |ap: &Value| {
        let name = field(ap, "ssid");
        let level = ap.get("signal_level").or_else(|| ap.get("rssi")).and_then(Value::as_i64).unwrap_or(-90);
        if name.is_empty() || best.get(&name).is_some_and(|(seen, _)| *seen >= level) {
            return;
        }
        let secured = ap.get("security").and_then(Value::as_str).is_none_or(|s| s != "OPEN");
        let net = Network { connected: name == ssid, hex: field(ap, "ssid_hex"), secured, bars: bars(level), ssid: name.clone() };
        best.insert(name, (level, net));
    };
    for ap in scan.as_ref().and_then(|s| s.get("scan")).and_then(Value::as_array).into_iter().flatten() {
        consider(ap);
    }
    // A scan can miss the network we are on; the status report still describes it.
    if let Some(s) = connected {
        consider(s);
    }

    let mut found: Vec<(i64, Network)> = best.into_values().collect();
    found.sort_by(|a, b| b.1.connected.cmp(&a.1.connected).then(b.0.cmp(&a.0)));
    (ssid, found.into_iter().take(MAX_NETWORKS).map(|(_, n)| n).collect())
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

pub fn demo_networks(connected: &str) -> Vec<Network> {
    [("HomeNetwork", 3), ("Guest", 4), ("SmartHome", 2)]
        .into_iter()
        .map(|(ssid, bars)| Network { ssid: ssid.into(), hex: String::new(), secured: true, bars, connected: ssid == connected })
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

/// The preferred Wi-Fi band ("a" or "b"), or empty when the core does not say.
pub(crate) async fn wifi_band(core: &coreapi::Core) -> String {
    core.request("get_network_cfg", json!({}))
        .await
        .ok()
        .and_then(|r| r.data.pointer("/wifi/band").and_then(Value::as_str).map(String::from))
        .unwrap_or_default()
}
