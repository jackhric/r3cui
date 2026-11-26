//! The demo: a simulated session for the desktop and the website, with no core behind it.

use crate::AppWindow;
use crate::app::{Later, UiAction};
use crate::app::model::Model;
use crate::art::{backdrop_pixels, poster_pixels, vibrant};
use crate::hardware::feedback;
use crate::input::{Button, slider};
use crate::Picker;
use crate::remote::wifi;
use crate::app::global::slide;
use crate::view::{finish_join, flash_volume, render, set_poster_accent, set_status, show_tracks};
use crate::view::format::{audio_format, split_track};

use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

pub(crate) fn apply_demo_settings(app: &AppWindow) {
    // Artwork for the demo's film, from local files, so previews show the real layout.
    let art = |var: &str| image::open(std::env::var_os(var)?).map_err(|e| tracing::warn!("{var}: {e}")).ok();
    let poster = art("UC_DEMO_POSTER");
    if let Some(p) = &poster {
        app.set_poster(slint::Image::from_rgba8(poster_pixels(p)));
        set_poster_accent(app, vibrant::accent(p));
    }
    if let Some(b) = art("UC_DEMO_FANART").or(poster) {
        app.set_backdrop(slint::Image::from_rgba8(backdrop_pixels(&b)));
    }
    app.set_haptics("On".into());
    app.set_screen_off_after("60 s".into());
    app.set_sleep_after("3 min".into());
}
pub(crate) fn demo_tracks(m: &Model, kind: Picker, subtitle: usize) -> (Vec<(String, String)>, usize) {
    if kind == Picker::Audio {
        let current = m.audio_tracks.iter().position(|t| *t == m.audio_track).unwrap_or(usize::MAX);
        (m.audio_tracks.iter().map(|t| split_track(t)).collect(), current)
    } else {
        // As many as a typical remux carries, so the picker has to scroll.
        let rows = [
            "Off", "English", "English SDH", "French · Forced", "French", "German", "Spanish", "Spanish (Latin America)",
            "Italian", "Portuguese (Brazil)", "Dutch", "Danish", "Finnish", "Norwegian", "Swedish", "Polish", "Czech",
            "Japanese", "Korean", "Chinese (Simplified)",
        ]
        .map(|l| (l.to_string(), String::new()));
        (rows.to_vec(), subtitle)
    }
}
/// No core at all: the keys and touch drive a local model, so the UI can be explored offline.
pub(crate) async fn run_demo(weak: &slint::Weak<AppWindow>, actions: &mut mpsc::UnboundedReceiver<UiAction>) {
    let mut m = Model::demo();
    let title = m.title.clone();
    set_status(weak, true, String::new());
    let overlay = Arc::new(AtomicU64::new(0));
    let (later_tx, mut later_rx) = mpsc::unbounded_channel::<Later>();
    let after = |delay: u64, later: Later| {
        let tx = later_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay)).await;
            let _ = tx.send(later);
        });
    };
    // The first try at a new network fails, so the failure page can be seen; the second works.
    let mut tried = String::new();
    let mut joining = String::new();
    let mut demo_subtitle = 1usize;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = tick.tick() => {
                if !m.paused && !m.title.is_empty() {
                    m.position = (m.position + 1.0).min(m.duration);
                }
            }
            Some(later) = later_rx.recv() => match later {
                Later::Wifi(_, networks) => (m.wifi, m.scanning) = (networks, false),
                Later::Switched => m.switching = false,
                Later::SendVolume | Later::Targets(_) => {}
                Later::JoinDone(ok) => {
                    if ok {
                        m.wifi_ssid = std::mem::take(&mut joining);
                        m.wifi = wifi::demo_networks(&m.wifi_ssid);
                        m.wifi.sort_by_key(|n| !n.connected);
                    }
                    finish_join(weak, ok);
                }
            },
            Some(action) = actions.recv() => match action {
                UiAction::WifiBand => m.wifi_band = if m.wifi_band == "a" { "b".into() } else { "a".into() },
                UiAction::WifiScan => {
                    m.scanning = true;
                    let mut found = wifi::demo_networks(&m.wifi_ssid);
                    found.sort_by_key(|n| !n.connected);
                    after(1400, Later::Wifi(m.wifi_ssid.clone(), found));
                }
                UiAction::WifiJoin { ssid, .. } => {
                    let ok = tried == ssid;
                    tried = ssid.clone();
                    joining = ssid;
                    after(1800, Later::JoinDone(ok));
                }
                UiAction::Backlight(..) | UiAction::Display { .. } => continue,
                UiAction::Seek(by) => m.position = (m.position + by as f64).clamp(0.0, m.duration),
                UiAction::OpenPicker(kind) => {
                    let (rows, current) = demo_tracks(&m, kind, demo_subtitle);
                    show_tracks(weak, rows, current);
                }
                UiAction::PickTrack(Picker::Audio, i) => {
                    if let Some(t) = m.audio_tracks.get(i) {
                        m.audio_track = t.clone();
                        m.media_format = audio_format(t);
                    }
                }
                UiAction::PickTrack(Picker::Subtitles, i) => demo_subtitle = i,
                UiAction::Slider(slider::Gesture::Steps(n)) => {
                    let before = m.volume;
                    m.volume = slide(m.volume, n, m.volume_max);
                    feedback::play(if m.volume == before { feedback::SLIDER_LIMIT } else { feedback::VOLUME });
                    m.muted = false;
                    flash_volume(weak, &overlay);
                }
                UiAction::Slider(slider::Gesture::DoubleTap) => {
                    feedback::play(feedback::SLIDER_MUTE);
                    m.muted = !m.muted;
                    flash_volume(weak, &overlay);
                }
                UiAction::DemoKey('c') => m.charging = !m.charging,
                UiAction::DemoKey('x') => m.demo_ha_down = !m.demo_ha_down,
                UiAction::SelectInput(name) => {
                    m.input = name;
                    m.switching = true;
                    after(1800, Later::Switched);
                }
                UiAction::PlayPause => m.paused = !m.paused,
                UiAction::Power(what) => tracing::info!("power action {what:?}"),
                UiAction::PowerHeld => m.power_on = !m.power_on,
                // Stand-in for the core's power-mode event.
                UiAction::DemoKey('z') => m.awake = !m.awake,
                UiAction::DemoKey(_) | UiAction::Integrations(_) => {}
                UiAction::Button(b) => match b {
                    Button::VolumeUp => { m.volume = (m.volume + 1).min(m.volume_max); m.muted = false; flash_volume(weak, &overlay); feedback::play(feedback::VOLUME); }
                    Button::VolumeDown => { m.volume = (m.volume - 1).max(0); m.muted = false; flash_volume(weak, &overlay); feedback::play(feedback::VOLUME); }
                    Button::Mute => { m.muted = !m.muted; flash_volume(weak, &overlay); }
                    Button::Play if m.title.is_empty() => m.title = title.clone(),
                    Button::Play => m.paused = !m.paused,
                    Button::Stop => m.title.clear(),
                    other => tracing::info!("key {}", other.name()),
                },
            },
        }
        m = m.with_demo_health();
        render(weak, &m);
    }
}
