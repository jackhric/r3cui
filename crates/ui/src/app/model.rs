//! Everything the screen shows, independent of where it came from (core or demo).

use crate::hardware::backlight;
use crate::remote::{health, wifi};

/// Everything the screen shows, independent of where it came from (core or demo).
#[derive(Clone, Debug, Default)]
pub(crate) struct Model {
    pub volume: i32,
    pub muted: bool,
    /// The input on now, by id.
    pub source: String,
    /// The inputs to choose from, in order, hidden ones left out.
    pub sources: Vec<Source>,
    /// The input on now has something in its UI slot: the home page is its stage.
    pub staged: bool,
    /// The input was just picked here and the receiver has not confirmed it yet.
    pub switching: bool,
    pub title: String,
    pub paused: bool,
    pub duration: f64,
    pub position: f64,
    pub media_format: String,
    pub media_sub: String,
    /// Demo only: the audio streams as named, and the one playing.
    pub audio_tracks: Vec<String>,
    pub audio_track: String,
    pub battery: i32,
    /// On the dock or a cable.
    pub charging: bool,
    /// The highest volume allowed (the receiver's own volume limit), on the same scale as `volume`.
    pub volume_max: i32,
    pub awake: bool,
    pub receiver_on: bool,
    pub wifi_ssid: String,
    pub wifi: Vec<wifi::Network>,
    /// The preferred Wi-Fi band as the core names it: "a" 5 GHz, "b" 2.4 GHz; empty when unknown.
    pub wifi_band: String,
    pub scanning: bool,
    pub status: Vec<health::StatusLine>,
    pub warning: String,
    pub keys: backlight::Ha,
    /// Demo only: pretend Home Assistant cannot be reached.
    pub demo_ha_down: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Source {
    pub id: String,
    pub name: String,
}
impl Model {
    pub fn demo() -> Self {
        Self {
            volume: 62,
            muted: false,
            source: "PLAYSTATION 5".into(),
            sources: ["ANDROID TV", "KODI", "PLAYSTATION 5", "XBOX 360"].into_iter().map(|s| Source { id: s.into(), name: s.into() }).collect(),
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
            wifi: wifi::demo_networks("HomeNetwork"),
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
        use drivers::api::{Health, Status};
        let link = |name: &str, up: bool| health::Link { name: name.into(), status: Status::new(if up { Health::Up } else { Health::Down }), in_use: true };
        self.staged = self.source.eq_ignore_ascii_case("kodi");
        (self.status, self.warning) = health::health(&self.wifi_ssid, &[link("Home Assistant", !self.demo_ha_down), link("Kodi", true)]);
        self
    }
}
