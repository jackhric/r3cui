//! The live session: the core connection, its entities, and the event loop that turns UI actions and core events
//! into commands and screen updates.

use crate::AppWindow;
use coreapi::{Attr, CoreEvent, Entity, EntityChange, MediaPlayerCmd, MediaState};
use crate::app::{Later, UiAction, settings};
use crate::app::entity::{list, number, pick_entity, text};
use crate::app::model::Model;
use crate::app::title::StableTitle;
use crate::art::{backdrop_pixels, fetch_image, poster_pixels, vibrant};
use crate::hardware::{backlight, feedback};
use crate::input::slider;
use crate::integrations::{ha_keys, health, kodi, library, system, wifi};
use crate::integrations::receiver::{slide, Receiver};
use crate::integrations::system::{load_settings, System};
use crate::integrations::wifi::Wifi;
use crate::view::{finish_join, flash_volume, render, set_poster_accent, set_status, show_tracks, ui};
use crate::view::format::{audio_format, episode, split_track};
use crate::input::{Action, KodiAction, ReceiverCmd, Target};
use crate::view::screen::Picker;

use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub(crate) const RECONNECT_DELAY: Duration = Duration::from_secs(2);
/// Whenever the driver's title changes, ask Kodi what is playing.
pub(crate) fn ask_title(session: &mut Session, tx: &mpsc::UnboundedSender<Later>) {
    let (Some(media), Some(base)) = (session.title.lookup(), kodi::endpoint()) else { return };
    let tx = tx.clone();
    tokio::task::spawn_blocking(move || match kodi::title(&base) {
        Ok(title) => {
            let _ = tx.send(Later::Title(media, title));
        }
        Err(e) => tracing::debug!("kodi title: {e:#}"),
    });
}
/// The driver reports the position in whole seconds, sent now and then; whenever it reports one, or a pause
/// or resume, Kodi's own position, to the millisecond, replaces it.
pub(crate) fn ask_position(session: &mut Session, tx: &mpsc::UnboundedSender<Later>) {
    if !std::mem::take(&mut session.timing) {
        return;
    }
    let Some(base) = kodi::endpoint() else { return };
    let tx = tx.clone();
    tokio::task::spawn_blocking(move || {
        let asked = Instant::now();
        match kodi::position(&base) {
            Ok(at) => {
                // Read about halfway through the round trip.
                let when = asked + asked.elapsed() / 2;
                let _ = tx.send(Later::Position(at, when));
            }
            Err(e) => tracing::debug!("kodi position: {e:#}"),
        }
    });
}
pub(crate) struct Session {
    /// In the order the core lists them, which picking the screen and receiver relies on.
    entities: Vec<Entity>,
    /// Kodi's media player.
    screen: Option<String>,
    receiver: Receiver,
    system: System,
    wifi: Wifi,
    /// integration id → whether the core reaches the device behind it.
    integrations: HashMap<String, bool>,
    position: f64,
    position_at: Instant,
    poster_url: String,
    title: StableTitle,
    /// The screen's position or play state has changed since Kodi was last asked for its exact position.
    timing: bool,
    /// What each row of the subtitle picker selects.
    subtitle_ids: Vec<Option<i64>>,
}
impl Session {
    fn entity(&self, id: &str) -> Option<&Entity> {
        self.entities.iter().find(|e| e.entity_id == id)
    }

    fn target(&self, t: Target) -> Option<&Entity> {
        let id = match t {
            Target::Screen => self.screen.as_deref(),
            Target::Volume => self.receiver.id.as_deref(),
        };
        id.and_then(|id| self.entity(id))
    }

    /// Merge an `entity_change` into the cached entity; returns whether it applied. A change to the screen's
    /// position or play state sets `timing`, for Kodi's exact position to be asked.
    fn apply_change(&mut self, change: &EntityChange) -> bool {
        let Some(id) = change.entity_id.as_deref() else { return false };
        let attrs = change.attributes();
        if attrs.is_empty() {
            return false;
        }
        let Some(entity) = self.entities.iter_mut().find(|e| e.entity_id == id) else { return false };
        let was_playing = entity.state() == MediaState::Playing;
        entity.apply(attrs);
        let playing = entity.state() == MediaState::Playing;
        if self.screen.as_deref() == Some(id) {
            self.timing |= attrs.contains_key(Attr::State.name()) || attrs.contains_key(Attr::MediaPosition.name());
            // The position runs on from the last report only while playing; a pause or a resume without a fresh
            // position restarts the count from where it stood, so a pause is not counted as time played.
            if attrs.contains_key(Attr::State.name()) {
                if playing != was_playing {
                    if was_playing {
                        self.position += self.position_at.elapsed().as_secs_f64();
                    }
                    self.position_at = Instant::now();
                }
            }
            if let Some(p) = attrs.get(Attr::MediaPosition.name()).and_then(Value::as_f64) {
                self.position = p;
                self.position_at = Instant::now();
            }
        }
        true
    }

    /// Fetch the artwork whenever Kodi reports a different image URL: the poster, and behind it
    /// the item's backdrop (the poster again when it has none), blurred.
    fn sync_poster(&mut self, weak: &slint::Weak<AppWindow>) {
        let url = text(self.target(Target::Screen), Attr::MediaImageUrl).to_string();
        if url == self.poster_url {
            return;
        }
        self.poster_url = url.clone();
        kodi::learn_from_image(&url);
        let weak = weak.clone();
        tokio::task::spawn_blocking(move || {
            let poster = (!url.is_empty()).then(|| fetch_image(&url).map_err(|e| tracing::warn!("poster {url}: {e:#}")).ok()).flatten();
            let fanart = kodi::endpoint()
                .filter(|_| !url.is_empty())
                .and_then(|base| kodi::playing(&base).map_err(|e| tracing::info!("kodi json-rpc: {e:#}")).ok())
                .map(|p| p.fanart)
                .filter(|f| !f.is_empty())
                .and_then(|f| fetch_image(&f).map_err(|e| tracing::warn!("fanart {f}: {e:#}")).ok());
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

    /// Feed the title filter with what Kodi reports now. Call before `model`.
    fn settle_title(&mut self) {
        let screen = self.target(Target::Screen);
        let media = format!("{}|{}", number(screen, Attr::MediaDuration), text(screen, Attr::MediaImageUrl));
        let (title, tracks) = (text(screen, Attr::MediaTitle).to_string(), text(screen, Attr::MediaArtist).to_string());
        let streams = list(screen, Attr::SoundModeList);
        // The driver reports the chapter playing as the media player's source.
        let chapter = text(screen, Attr::Source).to_string();
        self.title.feed(&media, &title, &tracks, &streams, &chapter, Instant::now());
    }

    fn model(&self) -> Model {
        let screen = self.target(Target::Screen);
        let receiver = self.target(Target::Volume).filter(|_| self.receiver.id != self.screen);
        let volume_from = receiver.or(screen);
        let state = screen.map_or(MediaState::Unknown, Entity::state);
        let paused = state != MediaState::Playing;
        let position = if paused { self.position } else { self.position + self.position_at.elapsed().as_secs_f64() };
        let link = |e: Option<&Entity>| self.integrations.get(e?.integration_id.as_deref()?).copied();
        let r = self.receiver.view(receiver, volume_from, &self.entities);
        let (status, warning) = health::health(health::Health {
            wifi_ssid: &self.wifi.ssid,
            home_assistant: link(receiver),
            receiver: receiver.map(Entity::state),
            kodi_link: link(screen),
            kodi: screen.is_some().then_some(state),
            on_kodi: r.source.is_empty() || r.source.eq_ignore_ascii_case("kodi"),
        });
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
            // The Kodi driver keeps the last title, artwork and position after a stop, so only the
            // playback state says whether anything is on.
            title: if state.is_active() { self.title.shown.clone() } else { String::new() },
            paused,
            duration: number(screen, Attr::MediaDuration),
            position,
            media_format: audio_format(text(screen, Attr::SoundMode)),
            media_sub: episode(text(screen, Attr::MediaType), text(screen, Attr::MediaArtist)),
            audio_tracks: list(screen, Attr::SoundModeList),
            audio_track: text(screen, Attr::SoundMode).to_string(),
            battery: self.system.battery,
            charging: self.system.charging,
            awake: self.system.awake,
            receiver_on: r.on,
            volume_max: r.volume_max,
        }
    }
}
pub(crate) async fn run(weak: &slint::Weak<AppWindow>, actions: &mut mpsc::UnboundedReceiver<UiAction>) -> anyhow::Result<()> {
    let cfg = coreapi::Config::from_env()?;
    set_status(weak, false, format!("Connecting to {}…", cfg.url));

    let (core, mut events) = coreapi::connect(cfg).await?;
    let entities = core.get_entities().await?;
    tracing::info!(entities = entities.len(), "get_entities");

    let screen = pick_entity("UC_SCREEN_ENTITY", &entities, |_| true);
    let (system, wifi, integrations) = tokio::join!(System::load(&core), Wifi::load(&core), health::integrations(&core));
    let mut session = Session {
        receiver: Receiver::new(pick_entity("UC_VOLUME_ENTITY", &entities, |e| e.has_feature("volume_up_down"))),
        system,
        wifi,
        integrations,
        position: 0.0,
        position_at: Instant::now(),
        poster_url: String::new(),
        subtitle_ids: Vec::new(),
        title: StableTitle::default(),
        timing: false,
        screen,
        entities,
    };
    load_settings(weak, &core).await;
    session.settle_title();
    session.sync_poster(weak);
    session.position = number(session.target(Target::Screen), Attr::MediaPosition);
    tracing::info!(screen = ?session.screen, receiver = ?session.receiver.id, "targets");
    if let Some(screen) = &session.screen {
        kodi::learn_from_entity(screen);
    }

    render(weak, &session.model());
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
            if let Some(entity) = session.target(Target::Volume) {
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
                session.settle_title();
                ask_title(&mut session, &later_tx);
                render(weak, &session.model());
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
                    Later::Subtitles(rows, current) => {
                        session.subtitle_ids = rows.iter().map(|r| r.0).collect();
                        let labels: Vec<String> = rows.into_iter().map(|r| r.1).collect();
                        show_tracks(weak, labels.iter().map(|l| (l.clone(), String::new())).collect(), current);
                    }
                    Later::Position(at, when) => {
                        (session.position, session.position_at) = (at, when);
                        render(weak, &session.model());
                    }
                    Later::Title(media, title) => {
                        session.title.answer(&media, title);
                        render(weak, &session.model());
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
                render(weak, &session.model());
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
                        render(weak, &session.model());
                    }
                    CoreEvent::BatteryStatus(b) => {
                        session.system.on_battery(&b);
                        render(weak, &session.model());
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
                            session.settle_title();
                            ask_title(&mut session, &later_tx);
                            ask_position(&mut session, &later_tx);
                            session.sync_poster(weak);
                            render(weak, &session.model());
                        }
                    }
                    CoreEvent::Other => {}
                }
            }
            Some(action) = actions.recv() => {
                // A Kodi action, kept for the poster wall's prediction.
                let mut pressed: Option<KodiAction> = None;
                let (target, cmd) = match action {
                    UiAction::Kodi(a) => {
                        pressed = Some(a);
                        (Target::Screen, a.command())
                    }
                    UiAction::SelectSource(name) => {
                        let cmd = session.receiver.select_source(name);
                        render(weak, &session.model());
                        (Target::Volume, cmd)
                    }
                    UiAction::Power(what) => {
                        // Not wired yet: a stray tap in a prototype should not switch the remote off.
                        tracing::info!("power action {what} (not sent)");
                        continue;
                    }
                    UiAction::PowerHeld => {
                        let on = session.model().receiver_on;
                        let cmd = session.receiver.toggle_power(on);
                        render(weak, &session.model());
                        (Target::Volume, cmd)
                    }
                    UiAction::Backlight(wanted) => {
                        keys.show(wanted);
                        continue;
                    }
                    UiAction::Display { brightness, auto } => {
                        system::set_display(&core, brightness, auto);
                        continue;
                    }
                    UiAction::Seek(by) => {
                        let m = session.model();
                        let to = (m.position + by as f64).clamp(0.0, m.duration.max(0.0));
                        (session.position, session.position_at) = (to, Instant::now());
                        render(weak, &session.model());
                        (Target::Screen, MediaPlayerCmd::Seek(to.round() as u32))
                    }
                    UiAction::OpenPicker(Picker::Audio) => {
                        let m = session.model();
                        let current = m.audio_tracks.iter().position(|t| *t == m.audio_track).unwrap_or(usize::MAX);
                        show_tracks(weak, m.audio_tracks.iter().map(|t| split_track(t)).collect(), current);
                        continue;
                    }
                    UiAction::OpenPicker(Picker::Subtitles) => {
                        show_tracks(weak, Vec::new(), usize::MAX);
                        let Some(base) = kodi::endpoint() else { continue };
                        let tx = later_tx.clone();
                        tokio::task::spawn_blocking(move || match kodi::playing(&base) {
                            Ok(p) => {
                                let mut rows = vec![(None, "Off".to_string())];
                                rows.extend(p.subtitles.into_iter().map(|(i, l)| (Some(i), l)));
                                let _ = tx.send(Later::Subtitles(rows, p.subtitle.map_or(0, |i| i + 1)));
                            }
                            Err(e) => tracing::warn!("subtitles: {e:#}"),
                        });
                        continue;
                    }
                    UiAction::PickTrack(Picker::Audio, i) => {
                        let Some(mode) = session.model().audio_tracks.get(i).cloned() else { continue };
                        (Target::Screen, MediaPlayerCmd::SelectSoundMode(mode))
                    }
                    UiAction::PickTrack(Picker::Subtitles, i) => {
                        let (Some(base), Some(id)) = (kodi::endpoint(), session.subtitle_ids.get(i).copied()) else { continue };
                        tokio::task::spawn_blocking(move || {
                            if let Err(e) = kodi::set_subtitle(&base, id) {
                                tracing::warn!("set subtitle: {e:#}");
                            }
                        });
                        continue;
                    }
                    UiAction::Slider(slider::Gesture::Steps(n)) => {
                        let (base, max) = (session.volume_base(), session.model().volume_max);
                        let v = slide(base, n, max);
                        feedback::play(if v == base { feedback::SLIDER_LIMIT } else { feedback::VOLUME });
                        session.receiver.slide_to(v);
                        flash_volume(weak, &overlay);
                        render(weak, &session.model());
                        let send = volume_gate.offer(v, Instant::now());
                        pace(&mut volume_gate, send, &session);
                        continue;
                    }
                    UiAction::Slider(slider::Gesture::DoubleTap) => {
                        feedback::play(feedback::SLIDER_MUTE);
                        flash_volume(weak, &overlay);
                        (Target::Volume, MediaPlayerCmd::MuteToggle)
                    }
                    UiAction::WifiBand => {
                        session.wifi.toggle_band(&core).await;
                        render(weak, &session.model());
                        // The remote reconnects on the new band; look again once it has.
                        refresh_wifi(Duration::from_secs(10));
                        continue;
                    }
                    UiAction::WifiScan => {
                        session.wifi.start_scan(&core);
                        render(weak, &session.model());
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
                        Action::Kodi(a) => {
                            pressed = Some(a);
                            (Target::Screen, a.command())
                        }
                        Action::Receiver(r) => {
                            flash_volume(weak, &overlay);
                            if matches!(r, ReceiverCmd::VolumeUp | ReceiverCmd::VolumeDown) {
                                feedback::play(feedback::VOLUME);
                            }
                            (Target::Volume, r.command())
                        }
                    },
                    UiAction::DemoKey(_) => continue,
                };
                let Some(entity) = session.target(target) else {
                    tracing::warn!("{}: no {target:?} entity", cmd.id());
                    continue;
                };
                let cmd = if cmd == MediaPlayerCmd::MuteToggle {
                    let (muted, entity) = (session.model().muted, entity.clone());
                    let cmd = session.receiver.toggle_mute(muted, &entity);
                    render(weak, &session.model());
                    cmd
                } else {
                    cmd
                };
                let Some(entity) = session.target(target) else { continue };
                if let Some(a) = pressed {
                    library::predict(weak, a);
                }
                commands.send(entity, cmd);
                if target == Target::Screen {
                    library::poke();
                }
            }
        }
    }
    Ok(())
}
pub(crate) type Queued = (MediaPlayerCmd, Instant);
/// The longest a command waits for the previous one's answer before going out anyway.
pub(crate) const PRESS_GAP: Duration = Duration::from_millis(40);
/// One queue per entity, for one core session: commands for an entity go out in the order they
/// were made, each after the previous one is answered or `PRESS_GAP` has passed.
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
                    // The next command waits for this one's answer, but not for long. The
                    // integration reorders commands that arrive together; a short gap keeps the
                    // order, and a Wi-Fi stall on the answer no longer holds up the next press.
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

