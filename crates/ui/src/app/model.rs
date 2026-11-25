//! Everything the screen shows, independent of where it came from (core or demo).

use crate::app::settings;
use crate::hardware::backlight;
use crate::integrations::net;


/// Everything the screen shows, independent of where it came from (core or demo).
#[derive(Clone, Debug, Default)]
pub(crate) struct Model {
    pub volume: i32,
    pub muted: bool,
    pub source: String,
    pub sources: Vec<String>,
    /// The input was just picked here and the receiver has not confirmed it yet.
    pub switching: bool,
    pub title: String,
    pub paused: bool,
    pub duration: f64,
    pub position: f64,
    pub media_format: String,
    pub media_sub: String,
    /// Kodi's audio streams as it names them, and the one playing.
    pub audio_tracks: Vec<String>,
    pub audio_track: String,
    pub battery: i32,
    /// On the dock or a cable.
    pub charging: bool,
    /// The highest volume the receiver allows (its own volume limit), on the same scale as `volume`.
    pub volume_max: i32,
    pub awake: bool,
    pub receiver_on: bool,
    pub wifi_ssid: String,
    pub wifi: Vec<net::Network>,
    /// The preferred Wi-Fi band as the core names it: "a" 5 GHz, "b" 2.4 GHz; empty when unknown.
    pub wifi_band: String,
    pub scanning: bool,
    pub status: Vec<net::StatusLine>,
    pub warning: String,
    pub keys: backlight::Ha,
    /// Demo only: pretend Home Assistant cannot be reached.
    pub demo_ha_down: bool,
}
impl Model {
    pub fn demo() -> Self {
        Self {
            volume: 62,
            muted: false,
            source: "PLAYSTATION 5".into(),
            sources: ["ANDROID TV", "Bluetooth", "HEOS Music", "KODI", "NET", "PLAYSTATION 5", "XBOX 360"]
                .into_iter()
                .filter(|s| !settings::get().hides(s))
                .map(String::from)
                .collect(),
            title: "Mad Max: Fury Road".into(),
            paused: false,
            duration: 7200.0,
            position: 2788.0,
            media_format: "DTS-HD MA 7.1".into(),
            audio_tracks: vec!["English DTS-HD MA 7.1 - DTS-HD MA 7.1".into(), "English Commentary - AAC-LC stereo".into(), "French - Dolby Digital 5.1".into()],
            audio_track: "English DTS-HD MA 7.1 - DTS-HD MA 7.1".into(),
            battery: 81,
            volume_max: 72,
            awake: true,
            receiver_on: true,
            wifi_ssid: "HomeNetwork".into(),
            wifi: net::demo_networks("HomeNetwork"),
            wifi_band: "b".into(),
            ..Default::default()
        }
        .with_demo_overrides()
        .with_demo_health()
    }

    /// Demo only: `UC_DEMO_TITLE`, `UC_DEMO_SOURCE` and `UC_DEMO_FORMAT` pick what the demo shows,
    /// for previews of other films.
    fn with_demo_overrides(mut self) -> Self {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        if let Some(t) = var("UC_DEMO_TITLE") {
            self.title = t;
        }
        if let Some(s) = var("UC_DEMO_SOURCE") {
            self.source = s;
        }
        if let Some(f) = var("UC_DEMO_FORMAT") {
            self.media_format = f;
        }
        self
    }

    /// Demo only: derive the status rows from the rest of the model.
    pub fn with_demo_health(mut self) -> Self {
        let kodi = if self.title.is_empty() { "ON" } else if self.paused { "PAUSED" } else { "PLAYING" };
        (self.status, self.warning) = net::health(net::Health {
            wifi_ssid: &self.wifi_ssid,
            home_assistant: Some(!self.demo_ha_down),
            receiver: Some(if self.receiver_on { "ON" } else { "OFF" }),
            kodi_link: Some(true),
            kodi: Some(kodi),
            on_kodi: self.on_kodi(),
        });
        self
    }

    /// With no receiver entity there is no input to follow, so the screen stays on Kodi.
    pub(crate) fn on_kodi(&self) -> bool {
        self.source.is_empty() || self.source.eq_ignore_ascii_case("kodi")
    }
}
