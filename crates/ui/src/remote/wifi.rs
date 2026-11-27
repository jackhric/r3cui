//! The remote's Wi-Fi: the network it's on, what a scan finds, the preferred band, and joining a network. The core
//! does the work; this keeps what it last said and turns its events into what the screen shows.

use coreapi::{WifiChange, WifiEvent};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long a join may take before it counts as failed.
const JOIN_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Default)]
pub(crate) struct Wifi {
    /// The network the remote is on; empty when disconnected.
    pub(crate) ssid: String,
    pub(crate) networks: Vec<Network>,
    /// "a" (5 GHz) or "b" (2.4 GHz); empty when the core doesn't say.
    pub(crate) band: String,
    pub(crate) scanning: bool,
    /// A join in progress: the network id once the core has given one, and when it started.
    joining: Option<(Option<i64>, Instant)>,
}

/// What a Wi-Fi event means for the screen.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Response {
    /// A join in progress has finished, and whether it worked.
    pub(crate) joined: Option<bool>,
    /// The network list is worth fetching again.
    pub(crate) refresh: bool,
}

impl Wifi {
    pub(crate) async fn load(core: &coreapi::Core) -> Self {
        let ((ssid, networks), band) = tokio::join!(scan(core), band(core));
        Self::new(ssid, networks, band)
    }

    pub(crate) fn new(ssid: String, networks: Vec<Network>, band: String) -> Self {
        Self { ssid, networks, band, ..Self::default() }
    }

    /// The results of a fetch.
    pub(crate) fn found(&mut self, ssid: String, networks: Vec<Network>) {
        (self.ssid, self.networks, self.scanning) = (ssid, networks, false);
    }

    pub(crate) fn on_event(&self, w: &WifiChange) -> Response {
        let joined = self.joining.and_then(|(ours, _)| match w.event {
            WifiEvent::Connected => Some(true),
            WifiEvent::WrongKey => Some(false),
            // A NETWORK_NOT_FOUND without our id is only a scan notice, not the outcome.
            WifiEvent::NetworkNotFound if w.id.is_some() && w.id == ours => Some(false),
            _ => None,
        });
        let refresh = matches!(w.event, WifiEvent::ScanCompleted | WifiEvent::Connected | WifiEvent::Disconnected);
        Response { joined, refresh }
    }

    pub(crate) fn join_timed_out(&self) -> bool {
        self.joining.is_some_and(|(_, since)| since.elapsed() > JOIN_TIMEOUT)
    }

    /// The join is over; whether one was in progress.
    pub(crate) fn end_join(&mut self) -> bool {
        self.joining.take().is_some()
    }

    /// Ask the core to join a network: by its raw name from a scan when there is one. Whether the core took it.
    pub(crate) async fn join(&mut self, core: &coreapi::Core, ssid: &str, hex: &str, password: &str) -> bool {
        let mut network = if hex.is_empty() { json!({ "ssid": ssid }) } else { json!({ "ssid_hex": hex }) };
        if !password.is_empty() {
            network["password"] = json!(password);
        }
        self.begin_join();
        match core.request("add_wifi_network", network).await {
            Ok(r) if r.is_ok() => {
                self.joining = Some((r.data.get("id").and_then(Value::as_i64), Instant::now()));
                true
            }
            other => {
                tracing::warn!("joining {ssid} could not start: {other:?}");
                false
            }
        }
    }

    pub(crate) fn begin_join(&mut self) {
        self.joining = Some((None, Instant::now()));
    }

    /// Switch the preferred band to the other one.
    pub(crate) async fn toggle_band(&mut self, core: &coreapi::Core) {
        let next = match self.band.as_str() {
            "a" => "b",
            "b" => "a",
            _ => return,
        };
        match core.request("set_network_cfg", json!({ "wifi": { "band": next } })).await {
            Ok(r) if r.is_ok() => {
                tracing::info!("wifi band now {next}");
                self.band = r.data.pointer("/wifi/band").and_then(Value::as_str).unwrap_or(next).to_string();
            }
            other => tracing::warn!("set_network_cfg band {next}: {other:?}"),
        }
    }

    pub(crate) fn start_scan(&mut self, core: &coreapi::Core) {
        self.scanning = true;
        let core = core.clone();
        tokio::spawn(async move {
            let _ = core.request("wifi_scan_start", json!({})).await;
        });
    }
}

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

fn bars(dbm: i64) -> i32 {
    match dbm {
        d if d >= -55 => 4,
        d if d >= -67 => 3,
        d if d >= -78 => 2,
        _ => 1,
    }
}

/// The connected network's name (empty when disconnected) and what the last scan found.
pub(crate) async fn scan(core: &coreapi::Core) -> (String, Vec<Network>) {
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

#[cfg(feature = "demo")]
pub fn demo_networks(connected: &str) -> Vec<Network> {
    let mut found: Vec<Network> = demo::data::WIFI
        .into_iter()
        .map(|(ssid, bars)| Network { ssid: ssid.into(), hex: String::new(), secured: true, bars, connected: ssid == connected })
        .collect();
    found.sort_by_key(|n| !n.connected);
    found
}

/// The preferred Wi-Fi band ("a" or "b"), or empty when the core does not say.
async fn band(core: &coreapi::Core) -> String {
    core.request("get_network_cfg", json!({}))
        .await
        .ok()
        .and_then(|r| r.data.pointer("/wifi/band").and_then(Value::as_str).map(String::from))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_outcomes() {
        let mut w = Wifi::default();
        let ev = |event, id| WifiChange { event, id };
        assert_eq!(w.on_event(&ev(WifiEvent::Connected, None)), Response { joined: None, refresh: true }, "no join in progress");
        w.joining = Some((Some(4), Instant::now()));
        assert_eq!(w.on_event(&ev(WifiEvent::NetworkNotFound, None)).joined, None, "a scan notice");
        assert_eq!(w.on_event(&ev(WifiEvent::NetworkNotFound, Some(4))).joined, Some(false));
        assert_eq!(w.on_event(&ev(WifiEvent::WrongKey, None)).joined, Some(false));
        assert!(w.end_join() && !w.end_join());
    }
}
