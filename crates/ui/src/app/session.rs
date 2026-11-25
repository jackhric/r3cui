//! The live session: the core connection for the remote's own state (power, battery, Wi-Fi, display, the key
//! backlight), the drivers for everything else, and the event loop that joins them to the screen. The frontend
//! talks to the host's ports and inputs; it never names a driver.

use crate::AppWindow;
use crate::app::global::{slide, Global};
use crate::app::integrations::{Effect, Integrations};
use crate::app::model::{Model, Source};
use crate::app::{Later, Settings, UiAction, settings};
use crate::art::{backdrop_pixels, fetch_image, poster_pixels, vibrant};
use crate::hardware::{backlight, feedback};
use crate::input::{slider, Route, VolumeKey};
use crate::remote::health::{self, Link};
use crate::remote::system::{self, load_settings, System};
use crate::remote::wifi::{self, Wifi};
use crate::view::library::Wall;
use crate::view::screen::{Page, Picker, Screen};
use crate::view::{finish_join, flash_volume, render, set_poster_accent, set_status, show_settings, show_tracks, ui};
use coreapi::CoreEvent;
use drivers::api::{KeyLight, KeyMode, Media, Stage, UiIntent, VolumeIntent};
use drivers::{Host, Input, Modules, Registry, Store};

use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub(crate) const RECONNECT_DELAY: Duration = Duration::from_secs(2);

/// What outlives a core reconnect: the drivers and what the screen keeps of them.
pub(crate) struct Frontend {
    pub(crate) host: Host,
    pub(crate) store: Store,
    global: Global,
    wall: Wall,
    settings: Integrations,
    /// The artwork on show, so it is fetched once per item.
    poster_url: String,
}

impl Frontend {
    /// The saved setup, or a first one from the old settings, or none. Within a tokio runtime.
    pub(crate) fn start(weak: slint::Weak<AppWindow>) -> Self {
        let store = store();
        let modules = match store.load() {
            Ok(Some(m)) => m,
            Ok(None) => {
                let first = settings::get().legacy_modules().unwrap_or_default();
                if first != Modules::default() {
                    let _ = store.save(&first).map_err(|e| tracing::warn!("{}: {e}", store.path.display()));
                }
                first
            }
            Err(e) => {
                tracing::error!("{e}: starting with no drivers");
                Modules::default()
            }
        };
        tracing::info!(drivers = ?modules.drivers.keys().collect::<Vec<_>>(), "modules from {}", store.path.display());
        Self { host: Host::new(Registry::builtin(), modules), store, global: Global::default(), wall: Wall::new(weak), settings: Integrations::default(), poster_url: String::new() }
    }

    /// Changes from the Integrations pages: saved, then applied.
    pub(crate) fn apply(&mut self, modules: Modules) {
        if let Err(e) = self.store.save(&modules) {
            tracing::error!("{}: {e}", self.store.path.display());
        }
        self.host.apply(modules);
    }

    fn current(&self) -> Option<Input> {
        self.host.current(&self.global.view(&self.host).source)
    }

    fn stage(&self) -> Stage {
        self.host.stage(self.current().as_ref())
    }
}

/// `UC_DATA_HOME` on the remote (the only place it may write), seeded from the archive's `config/`; `./config` on
/// the desktop.
fn store() -> Store {
    let config = std::env::var_os("UC_CONFIG_HOME").map_or_else(|| "config".into(), std::path::PathBuf::from);
    match std::env::var_os("UC_DATA_HOME") {
        Some(data) => Store::new(std::path::Path::new(&data), Some(&config)),
        None => Store::new(&config, None),
    }
}

struct Session<'a> {
    front: &'a mut Frontend,
    system: System,
    wifi: Wifi,
}

impl Session<'_> {
    /// Put everything on screen, and tell the UI slot whether it is showing.
    fn show(&mut self, weak: &slint::Weak<AppWindow>) {
        let current = self.front.current();
        let stage = self.front.host.stage(current.as_ref());
        let m = self.model(current.as_ref(), &stage);
        self.front.host.show(current.as_ref(), m.awake && m.receiver_on);
        self.front.wall.show(stage.library.as_ref());
        self.sync_poster(weak, stage.media.as_ref());
        show_settings(weak, self.front.settings.page(&self.front.host));
        render(weak, &m);
    }

    /// Settings → Integrations: what a pick or back leads to.
    fn settings(&mut self, weak: &slint::Weak<AppWindow>, action: Settings, later: &mpsc::UnboundedSender<Later>) {
        let front = &mut *self.front;
        let effect = match action {
            Settings::Open => {
                front.settings.open();
                Effect::Stay
            }
            Settings::Pick(i) => front.settings.pick(i, &front.host),
            Settings::Back => front.settings.back(),
            Settings::Text(text) => {
                let focus = front.settings.page(&front.host).focus as i32;
                ui(weak, move |app| app.go_to(Page::Integrations, focus));
                front.settings.text_entered(&text, &front.host).map_or(Effect::Stay, Effect::Apply)
            }
        };
        match effect {
            Effect::Stay => {}
            Effect::Apply(modules) => front.apply(modules),
            Effect::Load { driver, kind } => {
                if let Some(d) = front.host.driver(&driver) {
                    let later = later.clone();
                    tokio::spawn(async move {
                        let _ = later.send(Later::Targets(d.targets(kind).await));
                    });
                }
            }
            Effect::Text { title, caption, text } => {
                let focus = front.settings.page(&front.host).focus as i32;
                ui(weak, move |app| {
                    app.set_kb_purpose(1);
                    app.set_kb_title(title.into());
                    app.set_kb_caption(caption.into());
                    app.set_cfg_focus(focus);
                    app.invoke_kb_start(text.into());
                    app.go_to(Page::Password, 0);
                });
            }
            Effect::Leave => ui(weak, |app| app.go_to(Page::Menu, 1)),
        }
    }

    fn model(&self, current: Option<&Input>, stage: &Stage) -> Model {
        let host = &self.front.host;
        let g = self.front.global.view(host);
        let links: Vec<Link> =
            host.statuses().into_iter().map(|(id, name, status)| Link { in_use: host.in_use(&id, current), name, status }).collect();
        let (status, warning) = health::health(&self.wifi.ssid, &links);
        let now = Instant::now();
        let media = stage.media.as_ref();
        let (duration, position) = media.map_or((0.0, 0.0), |m| (m.total.as_secs_f64(), m.position_at(now).as_secs_f64()));
        Model {
            keys: keys(host.key_light()),
            wifi_ssid: self.wifi.ssid.clone(),
            wifi: self.wifi.networks.clone(),
            wifi_band: self.wifi.band.clone(),
            scanning: self.wifi.scanning,
            status,
            warning,
            demo_ha_down: false,
            volume: g.volume,
            has_volume: host.has_volume(),
            volume_max: g.volume_max,
            muted: g.muted,
            sources: host.inputs().into_iter().filter(|i| !i.hidden).map(|i| Source { id: i.id, name: i.name }).collect(),
            source: current.map_or(g.source, |i| i.id.clone()),
            switching: g.switching,
            staged: current.is_some_and(|i| i.ui.is_some()),
            title: media.map(|m| m.title.clone()).unwrap_or_default(),
            paused: media.is_none_or(|m| m.paused),
            duration,
            position,
            media_format: media.map(|m| m.format.clone()).unwrap_or_default(),
            media_sub: media.map(|m| m.subtitle.clone()).unwrap_or_default(),
            audio_tracks: Vec::new(),
            audio_track: String::new(),
            battery: self.system.battery,
            charging: self.system.charging,
            awake: self.system.awake,
            receiver_on: g.on,
        }
    }

    /// Fetch the artwork whenever what's playing changes: the poster, and behind it the item's backdrop (the poster
    /// again when it has none), blurred.
    fn sync_poster(&mut self, weak: &slint::Weak<AppWindow>, media: Option<&Media>) {
        let (url, fanart) = media.map(|m| (m.poster.clone(), m.backdrop.clone())).unwrap_or_default();
        if url == self.front.poster_url {
            return;
        }
        self.front.poster_url = url.clone();
        let weak = weak.clone();
        tokio::task::spawn_blocking(move || {
            let fetch = |u: &str| (!u.is_empty()).then(|| fetch_image(u).map_err(|e| tracing::warn!("artwork {u}: {e:#}")).ok()).flatten();
            let (poster, fanart) = (fetch(&url), fetch(&fanart));
            let backdrop = fanart.as_ref().or(poster.as_ref()).map(backdrop_pixels);
            let tint = poster.as_ref().and_then(vibrant::accent);
            let poster = poster.as_ref().map(poster_pixels);
            ui(&weak, move |app| {
                set_poster_accent(&app, tint);
                app.set_poster(poster.map(slint::Image::from_rgba8).unwrap_or_default());
                app.set_backdrop(backdrop.map(slint::Image::from_rgba8).unwrap_or_default());
            });
        });
    }
}

fn keys(light: Option<KeyLight>) -> backlight::Ha {
    let Some(k) = light else { return backlight::Ha::default() };
    let mode = match k.mode {
        KeyMode::Follow => backlight::Mode::Follow,
        KeyMode::Custom => backlight::Mode::Custom,
        KeyMode::Off => backlight::Mode::Off,
    };
    backlight::Ha { mode, colour: k.colour.map(|(r, g, b)| backlight::Rgb(r, g, b)) }
}

/// The picker's rows (label, detail) and the one on now; subtitles start with Off.
fn tracks(media: Option<&Media>, picker: Picker) -> (Vec<(String, String)>, usize) {
    let Some(m) = media else { return (vec![], usize::MAX) };
    let rows = |list: &[drivers::api::Track]| list.iter().map(|t| (t.label.clone(), t.detail.clone())).collect::<Vec<_>>();
    match picker {
        Picker::Audio => (rows(&m.audio), m.audio_current.unwrap_or(usize::MAX)),
        Picker::Subtitles => {
            let mut all = vec![("Off".to_string(), String::new())];
            all.extend(rows(&m.subtitles));
            (all, m.subtitle_current.map_or(0, |i| i + 1))
        }
    }
}

pub(crate) async fn run(weak: &slint::Weak<AppWindow>, actions: &mut mpsc::UnboundedReceiver<UiAction>, front: &mut Frontend) -> anyhow::Result<()> {
    let cfg = coreapi::Config::from_env()?;
    set_status(weak, false, format!("Connecting to {}…", cfg.url));
    let (core, mut events) = coreapi::connect(cfg).await?;
    let (system, wifi) = tokio::join!(System::load(&core), Wifi::load(&core));
    let mut session = Session { front, system, wifi };
    load_settings(weak, &core).await;
    session.show(weak);
    set_status(weak, true, String::new());

    // Presses made while offline would otherwise fire in a burst on reconnect.
    while actions.try_recv().is_ok() {}

    let overlay = Arc::new(AtomicU64::new(0));
    let (later_tx, mut later_rx) = mpsc::unbounded_channel::<Later>();
    let keys = backlight::Backlight::start(core.clone());
    let refresh_wifi = |delay: Duration| {
        let (core, tx) = (core.clone(), later_tx.clone());
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let (ssid, networks) = wifi::scan(&core).await;
            let _ = tx.send(Later::Wifi(ssid, networks));
        });
    };
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut changes = session.front.host.changes();
    let mut reported_volume = session.front.host.volume().map(|v| v.level);
    let mut volume_gate = slider::Pacer::new(Duration::from_millis(settings::get().volume_interval_ms.unwrap_or(250)), Duration::from_millis(600));
    // Spawns the timer a `Send::Later` asks for, or sends at once.
    let pace = |gate: &mut slider::Pacer, send: slider::Send, host: &Host| match send {
        slider::Send::Now(v) => {
            tracing::info!("slider volume {v} (from {} moves)", gate.take_merged());
            host.send_volume(VolumeIntent::Set(v));
        }
        slider::Send::Later(wait) => {
            let tx = later_tx.clone();
            tokio::spawn(async move {
                tokio::time::sleep(wait).await;
                let _ = tx.send(Later::SendVolume);
            });
        }
        slider::Send::Wait => {}
    };
    loop {
        tokio::select! {
            _ = tick.tick() => {
                if session.wifi.join_timed_out() {
                    let _ = later_tx.send(Later::JoinDone(false));
                }
                session.show(weak);
            }
            Ok(()) = changes.changed() => {
                let level = session.front.host.volume().map(|v| v.level);
                if level != reported_volume {
                    reported_volume = level;
                    if let Some(v) = level {
                        let send = volume_gate.reported(v, Instant::now());
                        pace(&mut volume_gate, send, &session.front.host);
                    }
                }
                session.show(weak);
            }
            Some(later) = later_rx.recv() => {
                match later {
                    Later::Wifi(ssid, networks) => session.wifi.found(ssid, networks),
                    Later::Switched => {}
                    Later::Targets(targets) => session.front.settings.targets_loaded(targets),
                    Later::SendVolume => {
                        let send = volume_gate.due(Instant::now());
                        pace(&mut volume_gate, send, &session.front.host);
                    }
                    Later::JoinDone(ok) => {
                        if session.wifi.end_join() {
                            finish_join(weak, ok);
                            refresh_wifi(Duration::from_millis(500));
                        }
                    }
                }
                session.show(weak);
            }
            ev = events.recv() => {
                let Some(ev) = ev else { break };
                tracing::debug!(msg = %ev.msg, cat = ?ev.cat, data = %ev.data, "event");
                match ev.typed() {
                    CoreEvent::PowerModeChange(p) => {
                        if session.system.on_power_mode(&p) {
                            keys.repaint();
                            // The core restores its own idea of the keys as it wakes; paint over it again once it has.
                            let keys = keys.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_millis(400)).await;
                                keys.repaint();
                            });
                        }
                        session.show(weak);
                    }
                    CoreEvent::BatteryStatus(b) => {
                        session.system.on_battery(&b);
                        session.show(weak);
                    }
                    CoreEvent::WifiChange(w) => {
                        let response = session.wifi.on_event(&w);
                        if let Some(ok) = response.joined {
                            let _ = later_tx.send(Later::JoinDone(ok));
                        }
                        if response.refresh {
                            refresh_wifi(Duration::ZERO);
                        }
                    }
                    CoreEvent::Other => {}
                }
            }
            Some(action) = actions.recv() => {
                let front = &mut *session.front;
                match action {
                    UiAction::Button(b) => match b.route() {
                        Some(Route::Volume(VolumeKey::Mute)) => {
                            flash_volume(weak, &overlay);
                            let muted = front.global.view(&front.host).muted;
                            front.global.toggle_mute(&front.host, muted);
                        }
                        Some(Route::Volume(key)) => {
                            flash_volume(weak, &overlay);
                            feedback::play(feedback::VOLUME);
                            front.host.send_volume(if key == VolumeKey::Up { VolumeIntent::Up } else { VolumeIntent::Down });
                        }
                        Some(Route::Remote(key)) if !front.host.key(front.current().as_ref(), key) => {
                            tracing::info!("{key:?}: nothing in this input's remote slot");
                        }
                        Some(Route::Remote(_)) => {}
                        None => {}
                    },
                    UiAction::PlayPause => {
                        front.host.send_ui(front.current().as_ref(), UiIntent::PlayPause);
                    }
                    UiAction::Seek(by) => {
                        front.host.send_ui(front.current().as_ref(), UiIntent::Seek(by));
                    }
                    UiAction::OpenPicker(p) => {
                        let (rows, current) = tracks(front.stage().media.as_ref(), p);
                        show_tracks(weak, rows, current);
                    }
                    UiAction::PickTrack(p, i) => {
                        let intent = match p {
                            Picker::Audio => UiIntent::Audio(i),
                            Picker::Subtitles => UiIntent::Subtitle(i.checked_sub(1)),
                        };
                        front.host.send_ui(front.current().as_ref(), intent);
                    }
                    UiAction::SelectSource(id) => front.global.select_source(&front.host, id),
                    UiAction::Power(what) => {
                        // Not wired yet: a stray tap in a prototype should not switch the remote off.
                        tracing::info!("power action {what} (not sent)");
                    }
                    UiAction::PowerHeld => {
                        let on = front.global.view(&front.host).on;
                        front.global.toggle_power(&front.host, on);
                    }
                    UiAction::Backlight(wanted) => keys.show(wanted),
                    UiAction::Display { brightness, auto } => system::set_display(&core, brightness, auto),
                    UiAction::Slider(slider::Gesture::Steps(n)) => {
                        let g = front.global.view(&front.host);
                        let v = slide(g.volume, n, g.volume_max);
                        feedback::play(if v == g.volume { feedback::SLIDER_LIMIT } else { feedback::VOLUME });
                        front.global.slide_to(v);
                        flash_volume(weak, &overlay);
                        let send = volume_gate.offer(v, Instant::now());
                        pace(&mut volume_gate, send, &front.host);
                    }
                    UiAction::Slider(slider::Gesture::DoubleTap) => {
                        feedback::play(feedback::SLIDER_MUTE);
                        flash_volume(weak, &overlay);
                        let muted = front.global.view(&front.host).muted;
                        front.global.toggle_mute(&front.host, muted);
                    }
                    UiAction::WifiBand => {
                        session.wifi.toggle_band(&core).await;
                        // The remote reconnects on the new band; look again once it has.
                        refresh_wifi(Duration::from_secs(10));
                    }
                    UiAction::WifiScan => {
                        session.wifi.start_scan(&core);
                        // SCAN_COMPLETED normally brings the results sooner; this covers a missed event.
                        refresh_wifi(Duration::from_secs(6));
                    }
                    UiAction::WifiJoin { ssid, hex, password } => {
                        if !session.wifi.join(&core, &ssid, &hex, &password).await {
                            let _ = later_tx.send(Later::JoinDone(false));
                        }
                    }
                    UiAction::Integrations(action) => session.settings(weak, action, &later_tx),
                    UiAction::DemoKey(_) => {}
                }
                session.show(weak);
            }
        }
    }
    Ok(())
}
