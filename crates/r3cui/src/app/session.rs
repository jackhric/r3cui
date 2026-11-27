//! The live session: the core connection for the remote's own state (power, battery, Wi-Fi, display, the key
//! backlight), the drivers for everything else, and the event loop that joins them to the screen. The frontend
//! talks to the host's ports and inputs; it never names a driver.

use crate::Playback;
use crate::Cfg;
use slint::ComponentHandle;
use crate::Kb;
use crate::AppWindow;
use crate::app::global::{slide, Global};
use crate::app::integrations::{Effect, Integrations};
use crate::app::model::{InputEntry, Model, NoInputs};
use crate::app::software::{self, Software};
use crate::app::voice::{self as voice_key, VoiceKey};
use crate::app::{Later, Settings, UiAction, settings};
use crate::art::{backdrop_pixels, fetch_image, poster_pixels, vibrant};
use crate::hardware::feedback;
use crate::input::{slider, Route, VolumeKey};
use crate::remote::health;
use crate::remote::system::{load_settings, System};
use crate::remote::link::Link;
use crate::remote::wifi::Wifi;
use crate::view::library::Wall;
use crate::view::screen::Screen;
use crate::{Menu, MenuItem, PageId};
use crate::{KbPurpose, Picker};
use crate::view::software::show as show_software;
use crate::view::{finish_join, flash_volume, render, set_poster_accent, set_status, show_settings, show_tracks, ui};
use coreapi::CoreEvent;
use r3cui_sdk::api::{Health, Media, PairError, Pairer, Stage, Status, UiIntent, VolumeIntent};
use host::{Host, Input, KnownTargets, Modules, Store, DEFAULT_INPUT};

use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub(crate) const RECONNECT_DELAY: Duration = Duration::from_secs(2);
const CLOCK_TICK: Duration = Duration::from_secs(1);
const DEFAULT_VOLUME_GAP_MS: u64 = 250;
const VOLUME_MAX_WAIT: Duration = Duration::from_millis(600);
const WIFI_REFRESH_AFTER_JOIN: Duration = Duration::from_millis(500);
const WIFI_REFRESH_AFTER_BAND_SWITCH: Duration = Duration::from_secs(10);
const WIFI_REFRESH_AFTER_SCAN: Duration = Duration::from_secs(6);
const KEYS_REPAINT_AFTER_WAKE: Duration = Duration::from_millis(400);

/// What outlives a core reconnect: the drivers and what the screen keeps of them.
pub(crate) struct Frontend {
    pub(crate) host: Host,
    /// Where the Integrations pages save; the demo keeps its setup to itself.
    store: Option<Store>,
    global: Global,
    wall: Wall,
    settings: Integrations,
    /// The pairing under way, between the device showing its code and the code being checked.
    pairer: Option<Box<dyn Pairer>>,
    /// The artwork on show, so it is fetched once per item.
    poster_url: String,
    /// r3cui's and the firmware's updates, and the web configurator.
    software: Software,
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
        let known = KnownTargets::load(&store.known_path());
        Self { host: Host::new(crate::app::registry::registry(), modules), store: Some(store), global: Global::default(), wall: Wall::new(weak), settings: Integrations::with_known(known), pairer: None, poster_url: String::new(), software: Software::new(crate::remote::selfupdate::current_version(), software::random_pin) }
    }

    /// The demo's drivers, set up in memory. Within a tokio runtime.
    #[cfg(feature = "demo")]
    pub(crate) fn demo(weak: slint::Weak<AppWindow>) -> Self {
        let modules = demo::modules();
        Self { host: Host::new(crate::app::registry::registry(), modules), store: None, global: Global::default(), wall: Wall::new(weak), settings: Integrations::default(), pairer: None, poster_url: String::new(), software: Software::new(crate::remote::selfupdate::current_version(), demo_pins()) }
    }

    /// Changes from the Integrations pages: saved, then applied.
    pub(crate) fn apply(&mut self, modules: Modules) {
        if let Some(store) = &self.store {
            if let Err(e) = store.save(&modules) {
                tracing::error!("{}: {e}", store.path.display());
            }
        }
        self.host.apply(modules);
    }

    fn current(&self) -> Option<Input> {
        self.host.current(&self.global.view(&self.host).input)
    }

    fn stage(&self) -> Stage {
        self.host.stage(self.current().as_ref())
    }
}

/// The made-up core's PINs, in turn.
#[cfg(feature = "demo")]
fn demo_pins() -> impl FnMut() -> String + Send + 'static {
    let mut next = demo::data::WEB_PINS.iter().cycle();
    move || next.next().map(|p| p.to_string()).unwrap_or_default()
}

/// `UC_DATA_HOME` on the remote (the only place it may write), seeded from the archive's `config/`; `./config` on
/// the desktop.
fn store() -> Store {
    let config = config_dir();
    match std::env::var_os("UC_DATA_HOME") {
        Some(data) => Store::new(std::path::Path::new(&data), Some(&config)),
        None => Store::new(&config, None),
    }
}

fn config_dir() -> std::path::PathBuf {
    std::env::var_os("UC_CONFIG_HOME").map_or_else(|| "config".into(), std::path::PathBuf::from)
}

/// Where r3cui may write what it keeps: `UC_DATA_HOME` on the remote, `./config` on the desktop.
pub(crate) fn data_dir() -> std::path::PathBuf {
    std::env::var_os("UC_DATA_HOME").map_or_else(config_dir, std::path::PathBuf::from)
}

struct Session<'a> {
    front: &'a mut Frontend,
    system: System,
    wifi: Wifi,
    voice: VoiceKey,
    /// r3cui's relay integration, to tell how a voice session ended; none in the demo.
    relay: Option<voicelink::LinkHandle>,
    /// Demo only: pretend every driver is unreachable.
    links_down: bool,
}

impl Session<'_> {
    /// Put everything on screen, and tell the UI slot whether it is showing.
    fn show(&mut self, weak: &slint::Weak<AppWindow>) {
        let current = self.front.current();
        let stage = self.front.host.stage(current.as_ref());
        for todo in self.voice.answered(self.front.host.voice().as_ref()) {
            if let (voice_key::Do::Report(msg), Some(relay)) = (todo, &self.relay) {
                relay.send(msg);
            }
        }
        let m = self.model(current.as_ref(), &stage);
        self.front.host.show(current.as_ref(), m.awake && m.power_on);
        self.front.wall.show(stage.library.as_ref());
        self.sync_poster(weak, stage.media.as_ref());
        show_settings(weak, self.front.settings.page(&self.front.host));
        show_software(weak, self.front.software.view(Instant::now(), self.system.battery));
        render(weak, &m);
    }

    fn voice_do(&mut self, link: &Link, later: &mpsc::UnboundedSender<Later>, todo: Vec<voice_key::Do>) {
        for d in todo {
            match d {
                voice_key::Do::Driver(intent) => {
                    if let r3cui_sdk::api::VoiceIntent::Audio { pcm, .. } = &intent {
                        crate::app::meter::hear(crate::app::meter::measure(pcm, voice_key::SAMPLE_RATE));
                    }
                    self.front.host.send_voice(intent);
                }
                voice_key::Do::CoreStart(s) => {
                    let link = link.clone();
                    tokio::spawn(async move { link.voice_start(s).await });
                }
                voice_key::Do::CoreEnd(s) => {
                    let link = link.clone();
                    tokio::spawn(async move { link.voice_end(s).await });
                }
                voice_key::Do::EndAfter(s, wait) => {
                    let later = later.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(wait).await;
                        let _ = later.send(Later::VoiceEndDue(s));
                    });
                }
                voice_key::Do::Report(msg) => {
                    if let Some(relay) = &self.relay {
                        relay.send(msg);
                    }
                }
            }
        }
    }

    /// Settings → Integrations: what a pick or back leads to.
    fn settings(&mut self, weak: &slint::Weak<AppWindow>, action: Settings, later: &mpsc::UnboundedSender<Later>) {
        let front = &mut *self.front;
        let effect = match action {
            Settings::Open => {
                front.settings.open();
                front.pairer = None;
                Effect::Stay
            }
            Settings::OpenForInputs => {
                front.settings.open_for_inputs(&front.host);
                front.pairer = None;
                let focus = front.settings.page(&front.host).focus as i32;
                ui(weak, move |app| app.go_to(PageId::Integrations, focus));
                Effect::Stay
            }
            Settings::Pick(i) => front.settings.pick(i, &front.host),
            Settings::Back => front.settings.back(),
            Settings::Text(text) => {
                let focus = front.settings.page(&front.host).focus as i32;
                ui(weak, move |app| app.go_to(PageId::Integrations, focus));
                front.settings.text_entered(&text, &front.host)
            }
        };
        self.run(weak, effect, later);
    }

    fn run(&mut self, weak: &slint::Weak<AppWindow>, effect: Effect, later: &mpsc::UnboundedSender<Later>) {
        let front = &mut *self.front;
        match effect {
            Effect::Stay => {}
            Effect::Apply(modules) => front.apply(modules),
            Effect::Load { driver, kind } => {
                front.settings.loading(&driver, kind);
                show_settings(weak, front.settings.page(&front.host));
                if let Some(d) = front.host.driver(&driver) {
                    let later = later.clone();
                    tokio::spawn(async move {
                        let targets = d.targets(kind).await;
                        let _ = later.send(Later::Targets { driver, kind, targets });
                    });
                }
            }
            Effect::Text { title, caption, text } => {
                let focus = front.settings.page(&front.host).focus as i32;
                ui(weak, move |app| {
                    app.global::<Kb>().set_purpose(KbPurpose::Text);
                    app.global::<Kb>().set_title(title.into());
                    app.global::<Kb>().set_caption(caption.into());
                    app.global::<Cfg>().set_focus(focus);
                    app.global::<Kb>().invoke_start(text.into());
                    app.go_to(PageId::Keyboard, 0);
                });
            }
            Effect::Code { title, caption, len } => {
                let focus = front.settings.page(&front.host).focus as i32;
                ui(weak, move |app| {
                    app.global::<Kb>().set_purpose(KbPurpose::Code);
                    app.global::<Kb>().set_title(title.into());
                    app.global::<Kb>().set_caption(caption.into());
                    app.global::<Kb>().set_min_len(len as i32);
                    app.global::<Cfg>().set_focus(focus);
                    app.global::<Kb>().invoke_start(Default::default());
                    app.go_to(PageId::Keyboard, 0);
                });
            }
            Effect::Pair { driver, apply } => {
                if let Some(modules) = apply {
                    front.apply(modules);
                }
                front.pairer = None;
                let start = front.host.driver(&driver).and_then(|d| d.pair());
                let later = later.clone();
                tokio::spawn(async move {
                    let started = match start {
                        Some(start) => start.await,
                        None => Err(PairError::Failed("Set it up first".into())),
                    };
                    let _ = later.send(Later::PairStarted(started));
                });
            }
            Effect::Finish { code } => match front.pairer.take() {
                Some(mut pairer) => {
                    let later = later.clone();
                    tokio::spawn(async move {
                        let checked = pairer.finish(&code).await;
                        let _ = later.send(Later::PairChecked(checked, pairer));
                    });
                }
                None => {
                    let lost = front.settings.pair_checked(Err(PairError::Failed("Pairing was interrupted".into())), &front.host);
                    self.run(weak, lost, later);
                }
            },
            Effect::Leave => ui(weak, |app| app.go_to(PageId::Menu, app.global::<Menu>().invoke_row(MenuItem::Integrations))),
        }
    }

    fn model(&self, current: Option<&Input>, stage: &Stage) -> Model {
        let host = &self.front.host;
        let g = self.front.global.view(host);
        let links: Vec<health::Link> = host
            .statuses()
            .into_iter()
            .map(|(id, name, status)| health::Link { in_use: host.in_use(&id, current), name, status: if self.links_down { Status::new(Health::Down) } else { status } })
            .collect();
        let (status, warning) = health::health(&self.wifi.ssid, &links);
        // Without a source, the one Default input is only worth listing when something fills its slots.
        let inputs: Vec<InputEntry> = host
            .inputs()
            .into_iter()
            .filter(|i| !i.hidden && !(i.id == DEFAULT_INPUT && i.remote.is_none() && i.ui.is_none()))
            .map(|i| InputEntry { icon: host.icon(&i), id: i.id, name: i.name })
            .collect();
        let source = host.modules().global.source.as_ref();
        let no_inputs = match source {
            _ if !inputs.is_empty() => NoInputs::No,
            Some(b) if host.inputs().is_empty() => {
                let link = links.iter().zip(host.statuses()).find(|(_, (id, ..))| *id == b.driver).map(|(l, _)| l);
                NoInputs::Source {
                    name: link.map_or_else(|| b.driver.clone(), |l| l.name.clone()),
                    icon: host.modules().drivers.get(&b.driver).and_then(|d| host.registry().spec(&d.kind)).map_or("", |s| s.icon),
                    address: host.address(&b.driver),
                    status: link.map_or_else(|| Status::new(Health::NotSetUp), |l| l.status.clone()),
                }
            }
            _ => NoInputs::SetUp { global: source.is_none() },
        };
        let now = Instant::now();
        let media = stage.media.as_ref();
        let (duration, position) = media.map_or((0.0, 0.0), |m| (m.total.as_secs_f64(), m.position_at(now).as_secs_f64()));
        Model {
            key_light: host.key_light().unwrap_or_default(),
            wifi_ssid: self.wifi.ssid.clone(),
            wifi: self.wifi.networks.clone(),
            wifi_band: self.wifi.band.clone(),
            scanning: self.wifi.scanning,
            status,
            warning,
            volume: g.volume,
            has_volume: host.has_volume(),
            volume_max: g.volume_max,
            muted: g.muted,
            inputs,
            no_inputs,
            has_voice: host.has_voice(),
            voice: self.voice.shown(host.voice().as_ref()).cloned(),
            input: current.map_or(g.input, |i| i.id.clone()),
            switching: g.switching,
            staged: current.is_some_and(|i| i.ui.is_some()),
            title: media.map(|m| m.title.clone()).unwrap_or_default(),
            paused: media.is_none_or(|m| m.paused),
            duration,
            position,
            media_format: media.map(|m| m.format.clone()).unwrap_or_default(),
            media_sub: media.map(|m| m.subtitle.clone()).unwrap_or_default(),
            app: stage.app.as_ref().map(|a| a.name.clone()).unwrap_or_default(),
            battery: self.system.battery,
            charging: self.system.charging,
            awake: self.system.awake,
            power_on: g.on,
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
                app.global::<Playback>().set_poster(poster.map(slint::Image::from_rgba8).unwrap_or_default());
                app.global::<Playback>().set_backdrop(backdrop.map(slint::Image::from_rgba8).unwrap_or_default());
            });
        });
    }
}

/// The picker's rows (label, detail) and the one on now; subtitles start with Off.
fn tracks(media: Option<&Media>, picker: Picker) -> (Vec<(String, String)>, usize) {
    let Some(m) = media else { return (vec![], usize::MAX) };
    let rows = |list: &[r3cui_sdk::api::Track]| list.iter().map(|t| (t.label.clone(), t.detail.clone())).collect::<Vec<_>>();
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
    let (core, events) = coreapi::connect(cfg).await?;
    let (system, wifi) = tokio::join!(System::load(&core), Wifi::load(&core));
    load_settings(weak, &core).await;
    if let Err(e) = core.announce_ui(&crate::remote::selfupdate::current_version().to_string()).await {
        tracing::warn!("telling the core which UI this is: {e}");
    }
    crate::remote::software::finish_pending_revoke(&core).await;
    // Presses made while offline would otherwise fire in a burst on reconnect.
    while actions.try_recv().is_ok() {}
    serve(weak, actions, front, Link::Core(core), events, system, wifi).await
}

/// The session proper, over a link to the remote: `run`'s core, or the demo's stand-in.
pub(crate) async fn serve(
    weak: &slint::Weak<AppWindow>,
    actions: &mut mpsc::UnboundedReceiver<UiAction>,
    front: &mut Frontend,
    link: Link,
    mut events: mpsc::Receiver<coreapi::Event>,
    system: System,
    wifi: Wifi,
) -> anyhow::Result<()> {
    let mut relay = link.relay();
    let mut session = Session { front, system, wifi, voice: VoiceKey::default(), relay: relay.as_ref().map(|r| r.handle.clone()), links_down: false };
    session.show(weak);
    set_status(weak, true, String::new());

    let overlay = Arc::new(AtomicU64::new(0));
    let (later_tx, mut later_rx) = mpsc::unbounded_channel::<Later>();
    software::run(session.front.software.on_connect(Instant::now()), &link, &later_tx, weak);
    let keys = link.backlight();
    let refresh_wifi = |delay: Duration| {
        let (link, tx) = (link.clone(), later_tx.clone());
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let (ssid, networks) = link.scan().await;
            let _ = tx.send(Later::Wifi(ssid, networks));
        });
    };
    let mut tick = tokio::time::interval(CLOCK_TICK);
    let mut changes = session.front.host.changes();
    let mut reported_volume = session.front.host.volume().map(|v| v.level);
    let mut volume_gate = slider::Pacer::new(Duration::from_millis(settings::get().volume_interval_ms.unwrap_or(DEFAULT_VOLUME_GAP_MS)), VOLUME_MAX_WAIT);
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
                software::run(session.front.software.tick(Instant::now()), &link, &later_tx, weak);
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
            Some(event) = next_relay(&mut relay) => {
                let todo = session.voice.relay(event);
                session.voice_do(&link, &later_tx, todo);
                session.show(weak);
            }
            Some(later) = later_rx.recv() => {
                match later {
                    Later::VoiceEndDue(s) => {
                        let todo = session.voice.end_due(s);
                        session.voice_do(&link, &later_tx, todo);
                    }
                    Later::Wifi(ssid, networks) => session.wifi.found(ssid, networks),
                    Later::Targets { driver, kind, targets } => {
                        let front = &mut *session.front;
                        if front.settings.targets_loaded(&driver, kind, targets) {
                            if let Some(store) = &front.store {
                                let path = store.known_path();
                                if let Err(e) = front.settings.known().save(&path) {
                                    tracing::warn!("{}: {e}", path.display());
                                }
                            }
                        }
                    }
                    Later::PairStarted(started) => {
                        let front = &mut *session.front;
                        let effect = match started {
                            Ok(pairer) => {
                                front.pairer = Some(pairer);
                                front.settings.pair_started(Ok(()), &front.host)
                            }
                            Err(e) => front.settings.pair_started(Err(e), &front.host),
                        };
                        session.run(weak, effect, &later_tx);
                    }
                    Later::PairChecked(checked, pairer) => {
                        let front = &mut *session.front;
                        if checked == Err(PairError::WrongCode) {
                            front.pairer = Some(pairer);
                        }
                        let effect = front.settings.pair_checked(checked, &front.host);
                        session.run(weak, effect, &later_tx);
                    }
                    Later::Software(event) => software::run(session.front.software.on_event(event, Instant::now()), &link, &later_tx, weak),
                    Later::SendVolume => {
                        let send = volume_gate.due(Instant::now());
                        pace(&mut volume_gate, send, &session.front.host);
                    }
                    Later::JoinDone(ok) => {
                        if session.wifi.end_join() {
                            finish_join(weak, ok);
                            refresh_wifi(WIFI_REFRESH_AFTER_JOIN);
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
                        if let (true, Some(keys)) = (session.system.on_power_mode(&p), keys.clone()) {
                            keys.repaint();
                            // The core restores its own idea of the keys as it wakes; paint over it again once it has.
                            tokio::spawn(async move {
                                tokio::time::sleep(KEYS_REPAINT_AFTER_WAKE).await;
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
                    CoreEvent::SoftwareUpdate(update) => {
                        let commands = session.front.software.on_event(software::Event::FirmwareUpdate(update), Instant::now());
                        software::run(commands, &link, &later_tx, weak);
                        session.show(weak);
                    }
                    CoreEvent::Assistant(_) | CoreEvent::Other => {}
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
                    UiAction::SelectInput(id) => front.global.select_input(&front.host, id),
                    UiAction::Power(what) => {
                        // Not wired yet: a stray tap in a prototype should not switch the remote off.
                        tracing::info!("power action {what:?} (not sent)");
                    }
                    UiAction::PowerHeld => {
                        let on = front.global.view(&front.host).on;
                        front.global.toggle_power(&front.host, on);
                    }
                    UiAction::Backlight(wanted) => {
                        if let Some(keys) = &keys {
                            keys.show(wanted);
                        }
                    }
                    UiAction::Display { brightness, auto } => link.set_display(brightness, auto),
                    UiAction::VoiceKey { down } => {
                        let todo = if down { session.voice.press(Instant::now()) } else { session.voice.release(Instant::now(), link.voice_grace()) };
                        session.voice_do(&link, &later_tx, todo);
                    }
                    UiAction::Wake => link.wake(),
                    UiAction::VoiceDismiss => session.voice.dismiss(session.front.host.voice().as_ref()),
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
                        link.toggle_band(&mut session.wifi).await;
                        // The remote reconnects on the new band; look again once it has.
                        refresh_wifi(link.settle(WIFI_REFRESH_AFTER_BAND_SWITCH));
                    }
                    UiAction::WifiScan => {
                        link.start_scan(&mut session.wifi);
                        // SCAN_COMPLETED normally brings the results sooner; this covers a missed event.
                        refresh_wifi(link.settle(WIFI_REFRESH_AFTER_SCAN));
                    }
                    UiAction::WifiJoin { ssid, hex, password } => {
                        if !link.join(&mut session.wifi, &ssid, &hex, &password, &later_tx).await {
                            let _ = later_tx.send(Later::JoinDone(false));
                        }
                    }
                    UiAction::Integrations(action) => session.settings(weak, action, &later_tx),
                    UiAction::Software(action) => software::run(front.software.act(action, Instant::now()), &link, &later_tx, weak),
                    #[cfg(feature = "demo")]
                    UiAction::DemoKey(key) => crate::app::demo::toggle(key, &mut session.system, &mut session.links_down),
                    #[cfg(not(feature = "demo"))]
                    UiAction::DemoKey(_) => {}
                }
                session.show(weak);
            }
        }
    }
    Ok(())
}

/// The relay's next event; never resolves without a relay.
async fn next_relay(relay: &mut Option<voicelink::Link>) -> Option<voicelink::LinkEvent> {
    match relay {
        Some(r) => r.events.recv().await,
        None => std::future::pending().await,
    }
}
