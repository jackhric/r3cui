//! Putting the model on screen, and the screen-side helpers the session and the demo share.

pub(crate) mod format;
pub(crate) mod idle;
pub(crate) mod keyboard;
pub(crate) mod lava;
pub(crate) mod screen;
pub(crate) mod stage;

use crate::view::screen::{Capture, JoinState, Page, Screen};
use crate::{AppWindow, Choice, Logos, SourceRow, StatusRow, Theme, WifiRow};

use crate::app::settings;
use crate::app::model::Model;
use crate::hardware::backlight;
use crate::integrations::receiver::{volume_db, volume_percent};
use crate::view::format::{band_label, clock, codec_badge, codec_logo, source_logo};
use crate::view::idle::poke;
use crate::view::stage::{capture, stage_change};
#[cfg(feature = "device")]
use crate::hardware::frames;

use slint::{ComponentHandle, Model as _, ModelRc, SharedString, VecModel};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub(crate) const VOLUME_OVERLAY: Duration = Duration::from_millis(1800);
/// The interface colour. Callable at any time from the UI thread.
pub(crate) fn set_accent(app: &AppWindow, color: slint::Color) {
    app.global::<Theme>().set_accent(color);
}
/// The colour picked from the current poster; while the film is on screen it stands in for the
/// input's accent, and the keys follow it.
pub(crate) fn set_poster_accent(app: &AppWindow, tint: Option<(u8, u8, u8)>) {
    if let Some((r, g, b)) = tint {
        tracing::info!("poster accent #{r:02X}{g:02X}{b:02X}");
    }
    app.set_poster_tinted(tint.is_some());
    if let Some((r, g, b)) = tint {
        app.set_poster_accent(slint::Color::from_rgb_u8(r, g, b));
    }
    // The poster arrives after the film is already showing, so take the colour up at once.
    let film_on_screen = app.get_receiver_on() && app.get_kodi() && !app.get_media_title().is_empty();
    if film_on_screen && tint.is_some() && std::env::var_os("UC_ACCENT").is_none() {
        set_accent(app, app.get_poster_accent());
    }
}
/// `#RRGGBB` or `RRGGBB`.
pub(crate) fn parse_color(s: &str) -> Option<slint::Color> {
    let hex = s.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(hex, 16).ok()?;
    Some(slint::Color::from_rgb_u8((v >> 16) as u8, (v >> 8) as u8, v as u8))
}
/// Fill the picker. `current` beyond the list marks none.
pub(crate) fn show_tracks(weak: &slint::Weak<AppWindow>, rows: Vec<(String, String)>, current: usize) {
    ui(weak, move |app| {
        let rows: Vec<Choice> = rows
            .into_iter()
            .enumerate()
            .map(|(i, (label, detail))| Choice { label: label.into(), detail: detail.into(), checked: i == current, ..Default::default() })
            .collect();
        let focus = rows.iter().position(|r| r.checked).unwrap_or(0) as i32;
        app.set_tracks(ModelRc::new(VecModel::from(rows)));
        if app.on_page(Page::Tracks) {
            app.set_focus_index(focus);
        }
    });
}
/// Leave the keyboard for the network list, or for the failure page.
pub(crate) fn finish_join(weak: &slint::Weak<AppWindow>, ok: bool) {
    ui(weak, move |app| {
        if !app.on_page(Page::Password) {
            return;
        }
        app.show_join(if ok { JoinState::Idle } else { JoinState::Failed });
        app.go_to(if ok { Page::Wifi } else { Page::JoinFailed }, 0);
    });
}
/// Show the volume overlay and hide it again once the keys have been quiet for a moment.
pub(crate) fn flash_volume(weak: &slint::Weak<AppWindow>, generation: &Arc<AtomicU64>) {
    let mine = generation.fetch_add(1, Ordering::SeqCst) + 1;
    ui(weak, |app| {
        if !app.get_volume_overlay() {
            capture(&app, Capture::World);
        }
        app.set_volume_overlay(true);
    });
    let (weak, generation) = (weak.clone(), generation.clone());
    tokio::spawn(async move {
        tokio::time::sleep(VOLUME_OVERLAY).await;
        if generation.load(Ordering::SeqCst) == mine {
            ui(&weak, |app| app.set_volume_overlay(false));
        }
    });
}
pub(crate) fn ui_call(weak: &slint::Weak<AppWindow>, f: impl FnOnce(AppWindow) + Send + 'static) {
    ui(weak, f)
}
pub(crate) fn ui(weak: &slint::Weak<AppWindow>, f: impl FnOnce(AppWindow) + Send + 'static) {
    let weak = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(app) = weak.upgrade() {
            f(app);
        }
    });
}
pub(crate) fn set_status(weak: &slint::Weak<AppWindow>, connected: bool, s: String) {
    ui(weak, move |app| {
        app.set_connected(connected);
        app.set_status(s.into());
    });
}
pub(crate) fn render(weak: &slint::Weak<AppWindow>, m: &Model) {
    let m = m.clone();
    ui(weak, move |app| apply(&app, &m));
}
/// A model change that alters what is on screen starts a transition; the playback position
/// ticking along does not count.
#[cfg(feature = "device")]
pub(crate) fn kick_if_changed(m: &Model) {
    use std::hash::{Hash, Hasher};
    thread_local!(static LAST: std::cell::Cell<u64> = const { std::cell::Cell::new(0) });
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (m.volume, m.muted, &m.source, &m.title, m.paused, m.awake, m.receiver_on, &m.warning, m.scanning, m.wifi.len()).hash(&mut h);
    format!("{:?}", m.keys).hash(&mut h);
    let now = h.finish();
    if LAST.with(|l| l.replace(now)) != now {
        frames::kick();
    }
}
/// Push a model into the window. UI thread only.
pub(crate) fn apply(app: &AppWindow, m: &Model) {
    stage_change(app, m, || apply_now(app, m));
}
pub(crate) fn apply_now(app: &AppWindow, m: &Model) {
    #[cfg(feature = "device")]
    kick_if_changed(m);
    let now = chrono::Local::now();
    let logos = app.global::<Logos>();

    app.set_clock(now.format("%H:%M").to_string().into());
    if std::env::var_os("UC_ACCENT").is_none() {
        let film_on_screen = m.receiver_on && m.on_kodi() && !m.title.is_empty() && app.get_poster_tinted();
        let accent = if film_on_screen { Some(app.get_poster_accent()) } else { settings::get().accent_for(&m.source).and_then(parse_color) };
        if let Some(accent) = accent {
            set_accent(app, accent);
        }
    }
    app.set_awake(m.awake);
    app.set_receiver_on(m.receiver_on);
    backlight::set_ha(m.keys);
    app.set_battery(m.battery);
    if m.charging != app.get_charging() {
        // Put on the charger: show the battery, and start counting down to dark from now.
        if m.charging {
            capture(app, Capture::World);
        }
        app.set_charge_card(m.charging);
        if m.charging {
            poke();
        } else {
            app.set_resting(false);
        }
    }
    app.set_charging(m.charging);
    // Shown as 0-100 of the receiver's limit; the dB figure stays the receiver's own.
    app.set_volume(volume_percent(m.volume, m.volume_max));
    app.set_volume_db(volume_db(m.volume).into());
    app.set_muted(m.muted);

    app.set_source(m.source.as_str().into());
    app.set_switching(m.switching);
    app.set_source_icon(source_logo(&logos, &m.source));
    let current = m.sources.iter().position(|s| *s == m.source).map_or(-1, |i| i as i32);
    if app.get_sources().iter().map(|r| r.name).ne(m.sources.iter().map(SharedString::from)) {
        let rows: Vec<SourceRow> = m
            .sources
            .iter()
            .map(|s| SourceRow { name: s.into(), icon: source_logo(&logos, s) })
            .collect();
        app.set_sources(ModelRc::new(VecModel::from(rows)));
    }
    app.set_source_index(current);
    let inputs: Vec<Choice> = m
        .sources
        .iter()
        .enumerate()
        .map(|(i, s)| Choice { label: s.into(), icon: source_logo(&logos, s), checked: i as i32 == current, ..Default::default() })
        .collect();
    if app.get_input_choices().iter().ne(inputs.iter().cloned()) {
        app.set_input_choices(ModelRc::new(VecModel::from(inputs)));
    }

    app.set_wifi_name(m.wifi_ssid.as_str().into());
    let wifi: Vec<WifiRow> = m
        .wifi
        .iter()
        .map(|n| WifiRow { ssid: (&n.ssid).into(), ssid_hex: (&n.hex).into(), secured: n.secured, bars: n.bars, connected: n.connected })
        .collect();
    if app.get_wifi_rows().iter().ne(wifi.iter().cloned()) {
        app.set_wifi_rows(ModelRc::new(VecModel::from(wifi)));
    }
    app.set_wifi_scanning(m.scanning);
    let mut networks: Vec<Choice> = m
        .wifi
        .iter()
        .map(|n| Choice {
            label: (&n.ssid).into(),
            value: if n.connected { "Connected".into() } else { SharedString::new() },
            locked: n.secured && !n.connected,
            bars: n.bars,
            ..Default::default()
        })
        .collect();
    networks.push(Choice { label: if m.scanning { "Scanning…" } else { "Scan again" }.into(), ..Default::default() });
    if let Some(band) = band_label(&m.wifi_band) {
        networks.push(Choice { label: "Band".into(), value: band.into(), ..Default::default() });
    }
    app.set_wifi_band_known(band_label(&m.wifi_band).is_some());
    if app.get_wifi_choices().iter().ne(networks.iter().cloned()) {
        app.set_wifi_choices(ModelRc::new(VecModel::from(networks)));
    }
    let status: Vec<StatusRow> =
        m.status.iter().map(|s| StatusRow { label: s.label.into(), value: (&s.value).into(), level: s.level }).collect();
    if app.get_status_rows().iter().ne(status.iter().cloned()) {
        app.set_status_rows(ModelRc::new(VecModel::from(status)));
    }
    let status_rows: Vec<Choice> =
        m.status.iter().map(|s| Choice { label: s.label.into(), value: (&s.value).into(), dot: s.level + 1, ..Default::default() }).collect();
    if app.get_status_choices().iter().ne(status_rows.iter().cloned()) {
        app.set_status_choices(ModelRc::new(VecModel::from(status_rows)));
    }
    app.set_home_warning(m.warning.as_str().into());

    app.set_kodi(m.on_kodi());
    app.set_media_title(m.title.as_str().into());
    app.set_media_sub(m.media_sub.as_str().into());
    app.set_paused(m.paused);
    let left = (m.duration - m.position).max(0.0);
    let known = m.duration > 0.0;
    app.set_progress(if known { (m.position / m.duration) as f32 } else { 0.0 });
    app.set_elapsed(if known { clock(m.position).into() } else { SharedString::new() });
    app.set_time_left(if known { format!("−{}", clock(left.ceil())).into() } else { SharedString::new() });
    app.set_ends_at(if known { (now + chrono::Duration::seconds(left as i64)).format("ends %H:%M").to_string().into() } else { SharedString::new() });
    let (logo, label) = codec_badge(&m.media_format);
    let mark = codec_logo(app, logo);
    app.set_media_format_logo(mark.path);
    app.set_media_format_logo_top(mark.top);
    app.set_media_format_logo_width(mark.width);
    app.set_media_format_logo_height(mark.height);
    app.set_media_format_logo_size(mark.size);
    app.set_media_format_logo_baseline(mark.baseline);
    app.set_media_format_label(label.into());
}
