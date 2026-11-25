//! The live session: the core connection, its entities, and the event loop that turns UI actions, the core's events
//! and Kodi's state into commands and screen updates. Kodi itself comes through r3cui's own client (`crates/kodi`);
//! the core gives the receiver, the remote's own state and its Wi-Fi.

use crate::AppWindow;
use crate::app::{Later, UiAction, settings};
use crate::app::entity::pick_entity;
use crate::app::model::Model;
use crate::art::{backdrop_pixels, fetch_image, poster_pixels, vibrant};
use crate::hardware::{backlight, feedback};
use crate::input::{slider, Action, ReceiverCmd};
use crate::integrations::{ha_keys, health, kodi_direct as kodi, system, wifi};
use crate::integrations::library::Library;
use crate::integrations::receiver::{slide, Receiver};
use crate::integrations::system::{load_settings, System};
use crate::integrations::wifi::Wifi;
use crate::view::{finish_join, flash_volume, render, set_poster_accent, set_status, show_tracks, ui};
use ::kodi::{KodiHandle, KodiService, KodiState};
use coreapi::{CoreEvent, Entity, MediaPlayerCmd};

use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, OnceCell};

pub(crate) const RECONNECT_DELAY: Duration = Duration::from_secs(2);

pub(crate) struct Session {
    /// In the order the core lists them, which picking the receiver relies on.
    entities: Vec<Entity>,
    receiver: Receiver,
    system: System,
    wifi: Wifi,
    /// integration id → whether the core reaches the device behind it.
    integrations: HashMap<String, bool>,
    library: Library,
    /// r3cui's own Kodi client; `None` when no Kodi is set up.
    kodi: Option<KodiHandle>,
    /// The artwork on show, so it is fetched once per item.
    poster_url: String,
}

impl Session {
    /// Put the model on screen, and let the poster wall show when it may.
    fn show(&self, weak: &slint::Weak<AppWindow>) {
        let m = self.model();
        self.library.allow(m.wall_allowed());
        render(weak, &m);
    }

    fn entity(&self, id: &str) -> Option<&Entity> {
        self.entities.iter().find(|e| e.entity_id == id)
    }

    fn receiver_entity(&self) -> Option<&Entity> {
        self.receiver.id.as_deref().and_then(|id| self.entity(id))
    }

    fn kodi_state(&self) -> KodiState {
        self.kodi.as_ref().map(|k| k.state().borrow().clone()).unwrap_or_default()
    }

    /// Merge an `entity_change` into the cached entity; returns whether it applied.
    fn apply_change(&mut self, change: &coreapi::EntityChange) -> bool {
        let Some(id) = change.entity_id.as_deref() else { return false };
        let attrs = change.attributes();
        let Some(entity) = self.entities.iter_mut().find(|e| e.entity_id == id) else { return false };
        entity.apply(attrs);
        !attrs.is_empty()
    }

    /// Fetch the artwork whenever what's playing changes: the poster, and behind it the item's backdrop (the poster
    /// again when it has none), blurred.
    fn sync_poster(&mut self, weak: &slint::Weak<AppWindow>) {
        let (url, fanart) = self.kodi.as_ref().map(|k| kodi::art(k, &k.state().borrow())).unwrap_or_default();
        if url == self.poster_url {
            return;
        }
        self.poster_url = url.clone();
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

    /// The level the next slider step starts from.
    fn volume_base(&self) -> i32 {
        self.model().volume
    }

    fn model(&self) -> Model {
        let receiver = self.receiver_entity();
        let link = |e: Option<&Entity>| self.integrations.get(e?.integration_id.as_deref()?).copied();
        let r = self.receiver.view(receiver, receiver, &self.entities);
        let kodi_state = self.kodi_state();
        let (kodi_link, kodi_doing) = kodi::health(&kodi_state);
        let (status, warning) = health::health(health::Health {
            wifi_ssid: &self.wifi.ssid,
            home_assistant: link(receiver),
            receiver: receiver.map(Entity::state),
            kodi_link,
            kodi: kodi_doing,
            on_kodi: r.source.is_empty() || r.source.eq_ignore_ascii_case("kodi"),
        });
        let np = kodi::now_playing(&kodi_state);
        Model {
            keys: ha_keys::keys(&self.entities),
            wifi_ssid: self.wifi.ssid.clone(),
            wifi: self.wifi.networks.clone(),
            wifi_band: self.wifi.band.clone(),
            scanning: self.wifi.scanning,
            status,
            warning,
            demo_ha_down: false,
            volume: r.volume,
            muted: r.muted,
            source: r.source,
            sources: r.sources,
            switching: r.switching,
            title: np.title,
            paused: np.paused,
            duration: np.duration,
            position: np.position,
            media_format: np.media_format,
            media_sub: np.media_sub,
            audio_tracks: Vec::new(),
            audio_track: String::new(),
            battery: self.system.battery,
            charging: self.system.charging,
            awake: self.system.awake,
            receiver_on: r.on,
            volume_max: r.volume_max,
        }
    }
}

pub(crate) async fn run(
    weak: &slint::Weak<AppWindow>,
    actions: &mut mpsc::UnboundedReceiver<UiAction>,
    library: &Library,
    kodi_client: &OnceCell<KodiHandle>,
) -> anyhow::Result<()> {
    let cfg = coreapi::Config::from_env()?;
    set_status(weak, false, format!("Connecting to {}…", cfg.url));

    let (core, mut events) = coreapi::connect(cfg).await?;
    let entities = core.get_entities().await?;
    tracing::info!(entities = entities.len(), "get_entities");

    // r3cui's own Kodi client, started once and kept across core reconnects. Until Kodi's address is in settings,
    // it comes from the core's Kodi entity, if there is one.
    let kodi = match kodi_client.get() {
        Some(k) => Some(k.clone()),
        None => match kodi::config(pick_entity("UC_SCREEN_ENTITY", &entities, |_| true).as_deref()) {
            Some(cfg) => Some(kodi_client.get_or_init(|| async move { KodiService::start(cfg) }).await.clone()),
            None => {
                tracing::warn!("no Kodi to follow: set kodi.host in settings.json");
                None
            }
        },
    };
    if let Some(k) = &kodi {
        library.attach(k.clone());
    }
    let mut kodi_state = kodi.as_ref().map(KodiHandle::state);
    let (system, wifi, integrations) = tokio::join!(System::load(&core), Wifi::load(&core), health::integrations(&core));
    let mut session = Session {
        receiver: Receiver::new(pick_entity("UC_VOLUME_ENTITY", &entities, |e| e.has_feature("volume_up_down"))),
        system,
        wifi,
        integrations,
        library: library.clone(),
        kodi,
        poster_url: String::new(),
        entities,
    };
    load_settings(weak, &core).await;
    session.sync_poster(weak);
    tracing::info!(kodi = ?session.kodi.as_ref().map(|k| k.config().host.clone()), receiver = ?session.receiver.id, "targets");

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
    let mut ticks = 0u64;
    let commands = Commands::new(core.clone());
    let mut volume_gate = slider::Pacer::new(Duration::from_millis(settings::get().volume_interval_ms.unwrap_or(250)), Duration::from_millis(600));
    // Spawns the timer a `Send::Later` asks for, or sends at once.
    let pace = |gate: &mut slider::Pacer, send: slider::Send, session: &Session| match send {
        slider::Send::Now(v) => {
            tracing::info!("slider volume {v} (from {} moves)", gate.take_merged());
            if let Some(entity) = session.receiver_entity() {
                commands.send(entity, MediaPlayerCmd::Volume(v));
            }
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
                ticks += 1;
                if ticks % 5 == 0 {
                    let (core, tx) = (core.clone(), later_tx.clone());
                    tokio::spawn(async move {
                        let _ = tx.send(Later::Integrations(health::integrations(&core).await));
                    });
                }
                // Entities shared with the remote after startup only show up by asking again.
                if ticks % 20 == 0 {
                    let (core, tx) = (core.clone(), later_tx.clone());
                    tokio::spawn(async move {
                        if let Ok(entities) = core.get_entities().await {
                            let _ = tx.send(Later::Entities(entities));
                        }
                    });
                }
                if session.wifi.join_timed_out() {
                    let _ = later_tx.send(Later::JoinDone(false));
                }
                session.show(weak);
            }
            Some(()) = async {
                match kodi_state.as_mut() {
                    Some(rx) => rx.changed().await.ok(),
                    None => std::future::pending().await,
                }
            } => {
                session.sync_poster(weak);
                session.show(weak);
            }
            Some(later) = later_rx.recv() => {
                match later {
                    Later::Wifi(ssid, networks) => session.wifi.found(ssid, networks),
                    Later::Integrations(map) => session.integrations = map,
                    Later::Entities(entities) => {
                        for e in entities {
                            if session.entity(&e.entity_id).is_none() {
                                tracing::info!("new entity {}", e.entity_id);
                                session.entities.push(e);
                            }
                        }
                    }
                    Later::Switched => {}
                    Later::SendVolume => {
                        let send = volume_gate.due(Instant::now());
                        pace(&mut volume_gate, send, &session);
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
                            // The core restores its own idea of the keys as it wakes; paint over it
                            // again once it has.
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
                    CoreEvent::EntityChange(change) => {
                        if session.receiver.id.is_some() && session.receiver.id == change.entity_id {
                            if let Some(v) = change.attributes().get("volume").and_then(Value::as_f64) {
                                let send = volume_gate.reported(v.round() as i32, Instant::now());
                                pace(&mut volume_gate, send, &session);
                            }
                        }
                        if session.apply_change(&change) {
                            session.show(weak);
                        }
                    }
                    CoreEvent::Other => {}
                }
            }
            Some(action) = actions.recv() => {
                // Kodi's keys, seeking and the track pickers go to Kodi; the rest is the receiver's, the remote's or
                // the Wi-Fi's.
                let kodi_key = match &action {
                    UiAction::Kodi(a) => Some(*a),
                    UiAction::Button(b) => match b.action() {
                        Action::Kodi(a) => Some(a),
                        Action::Receiver(_) => None,
                    },
                    _ => None,
                };
                if let Some(a) = kodi_key {
                    match &session.kodi {
                        Some(k) => {
                            session.library.pressed(Some(a));
                            kodi::press(k, a);
                        }
                        None => tracing::info!("{a:?}: no Kodi"),
                    }
                    continue;
                }
                let cmd = match action {
                    UiAction::Kodi(_) | UiAction::DemoKey(_) => continue,
                    UiAction::Seek(by) => {
                        if let Some(k) = &session.kodi {
                            kodi::seek_by(k, by);
                        }
                        continue;
                    }
                    UiAction::OpenPicker(p) => {
                        let (rows, current) = kodi::tracks(&session.kodi_state(), p);
                        show_tracks(weak, rows, current);
                        continue;
                    }
                    UiAction::PickTrack(p, i) => {
                        if let Some(k) = &session.kodi {
                            kodi::pick(k, p, i);
                        }
                        continue;
                    }
                    UiAction::SelectSource(name) => {
                        let cmd = session.receiver.select_source(name);
                        session.show(weak);
                        cmd
                    }
                    UiAction::Power(what) => {
                        // Not wired yet: a stray tap in a prototype should not switch the remote off.
                        tracing::info!("power action {what} (not sent)");
                        continue;
                    }
                    UiAction::PowerHeld => {
                        let on = session.model().receiver_on;
                        let cmd = session.receiver.toggle_power(on);
                        session.show(weak);
                        cmd
                    }
                    UiAction::Backlight(wanted) => {
                        keys.show(wanted);
                        continue;
                    }
                    UiAction::Display { brightness, auto } => {
                        system::set_display(&core, brightness, auto);
                        continue;
                    }
                    UiAction::Slider(slider::Gesture::Steps(n)) => {
                        let (base, max) = (session.volume_base(), session.model().volume_max);
                        let v = slide(base, n, max);
                        feedback::play(if v == base { feedback::SLIDER_LIMIT } else { feedback::VOLUME });
                        session.receiver.slide_to(v);
                        flash_volume(weak, &overlay);
                        session.show(weak);
                        let send = volume_gate.offer(v, Instant::now());
                        pace(&mut volume_gate, send, &session);
                        continue;
                    }
                    UiAction::Slider(slider::Gesture::DoubleTap) => {
                        feedback::play(feedback::SLIDER_MUTE);
                        flash_volume(weak, &overlay);
                        MediaPlayerCmd::MuteToggle
                    }
                    UiAction::WifiBand => {
                        session.wifi.toggle_band(&core).await;
                        session.show(weak);
                        // The remote reconnects on the new band; look again once it has.
                        refresh_wifi(Duration::from_secs(10));
                        continue;
                    }
                    UiAction::WifiScan => {
                        session.wifi.start_scan(&core);
                        session.show(weak);
                        // SCAN_COMPLETED normally brings the results sooner; this covers a missed event.
                        refresh_wifi(Duration::from_secs(6));
                        continue;
                    }
                    UiAction::WifiJoin { ssid, hex, password } => {
                        if !session.wifi.join(&core, &ssid, &hex, &password).await {
                            let _ = later_tx.send(Later::JoinDone(false));
                        }
                        continue;
                    }
                    UiAction::Button(b) => match b.action() {
                        Action::Receiver(r) => {
                            flash_volume(weak, &overlay);
                            if matches!(r, ReceiverCmd::VolumeUp | ReceiverCmd::VolumeDown) {
                                feedback::play(feedback::VOLUME);
                            }
                            r.command()
                        }
                        Action::Kodi(_) => continue,
                    },
                };
                // Everything left goes to the receiver.
                let Some(entity) = session.receiver_entity().cloned() else {
                    tracing::warn!("{}: no receiver", cmd.id());
                    continue;
                };
                let cmd = if cmd == MediaPlayerCmd::MuteToggle {
                    let cmd = session.receiver.toggle_mute(session.model().muted, &entity);
                    session.show(weak);
                    cmd
                } else {
                    cmd
                };
                commands.send(&entity, cmd);
            }
        }
    }
    Ok(())
}
pub(crate) type Queued = (MediaPlayerCmd, Instant);
/// The longest a command waits for the previous one's answer before going out anyway.
pub(crate) const PRESS_GAP: Duration = Duration::from_millis(40);
/// One queue per entity, for one core session: commands for an entity (the receiver) go out in the order they were
/// made, each after the previous one is answered or `PRESS_GAP` has passed.
pub(crate) struct Commands {
    core: coreapi::Core,
    queues: RefCell<HashMap<String, mpsc::UnboundedSender<Queued>>>,
}
impl Commands {
    fn new(core: coreapi::Core) -> Self {
        Self { core, queues: RefCell::new(HashMap::new()) }
    }

    fn send(&self, entity: &Entity, cmd: MediaPlayerCmd) {
        let id = entity.entity_id.clone();
        let mut queues = self.queues.borrow_mut();
        let tx = queues.entry(id.clone()).or_insert_with(|| {
            let (tx, mut rx) = mpsc::unbounded_channel::<Queued>();
            let (core, name) = (self.core.clone(), entity.name().to_string());
            tokio::spawn(async move {
                while let Some((cmd, made)) = rx.recv().await {
                    let waited = made.elapsed();
                    let sent = Instant::now();
                    let mut answer = core.submit_command(&id, &cmd).await;
                    // The next command waits for this one's answer, but not for long: integrations can reorder
                    // commands that arrive together (the core's Kodi integration did), a short gap keeps the order,
                    // and a Wi-Fi stall on the answer doesn't hold up the next press.
                    let quick = match &mut answer {
                        Ok(rx) => tokio::time::timeout(PRESS_GAP, rx).await.ok(),
                        Err(_) => None,
                    };
                    let name = name.clone();
                    tokio::spawn(async move {
                        let reply = match (quick, answer) {
                            (Some(reply), _) => reply.map_err(|_| "no answer".to_string()),
                            (None, Ok(rx)) => rx.await.map_err(|_| "no answer".to_string()),
                            (None, Err(e)) => Err(format!("{e:#}")),
                        };
                        let outcome = match reply {
                            Ok(r) if r.is_ok() => "ok".to_string(),
                            Ok(r) => r.data.get("message").and_then(Value::as_str).unwrap_or("failed").to_string(),
                            Err(e) => e,
                        };
                        tracing::info!("{name} · {} — {outcome} in {} ms (queued {} ms)", cmd.id(), sent.elapsed().as_millis(), waited.as_millis());
                    });
                }
            });
            tx
        });
        let _ = tx.send((cmd, Instant::now()));
    }
}

