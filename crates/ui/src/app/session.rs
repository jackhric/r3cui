//! The live session: the core connection, its entities, and the event loop that turns UI actions and core events
//! into commands and screen updates.

use crate::AppWindow;
use crate::app::optimistic::Optimistic;
use coreapi::{Attr, CoreEvent, Entity, EntityChange, MediaState, PowerMode, WifiEvent};
use crate::app::{Later, UiAction, settings};
use crate::app::entity::{list, number, pick_entity, text};
use crate::app::model::Model;
use crate::app::title::StableTitle;
use crate::art::{backdrop_pixels, fetch_image, poster_pixels, vibrant};
use crate::hardware::{backlight, feedback};
use crate::input::slider;
use crate::integrations::{kodi, library, net};
use crate::integrations::net::wifi_band;
use crate::integrations::receiver::{MAX_VOLUME, slide};
use crate::view::{finish_join, flash_volume, render, set_poster_accent, set_status, show_tracks, ui};
use crate::view::format::{audio_format, duration_label, episode, split_track};
use crate::input::{Action, KodiAction, ReceiverCmd, Target};
use crate::view::screen::Picker;

use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub(crate) const RECONNECT_DELAY: Duration = Duration::from_secs(2);
pub(crate) const JOIN_TIMEOUT: Duration = Duration::from_secs(45);
pub(crate) const SLIDER_HOLD: Duration = Duration::from_millis(1500);
/// The longest the slider's level is shown while the receiver has not yet reported it.
pub(crate) const SLIDER_SETTLE: Duration = Duration::from_secs(6);
/// How long an input picked here is shown before the receiver's own report wins again.
pub(crate) const SWITCH_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a power change made here is believed before the receiver's own report wins again.
pub(crate) const POWER_TIMEOUT: Duration = Duration::from_secs(120);
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
    screen: Option<String>,
    volume: Option<String>,
    position: f64,
    position_at: Instant,
    battery: i32,
    poster_url: String,
    awake: bool,
    wifi_ssid: String,
    wifi: Vec<net::Network>,
    wifi_band: String,
    scanning: bool,
    integrations: HashMap<String, bool>,
    title: StableTitle,
    /// The screen's position or play state has changed since Kodi was last asked for its exact position.
    timing: bool,
    /// The network id of a join in progress, and when it started.
    joining: Option<(Option<i64>, Instant)>,
    /// What each row of the subtitle picker selects.
    subtitle_ids: Vec<Option<i64>>,
    /// The level the slider last asked for. The receiver takes a moment to report it back, and
    /// steps taken meanwhile must build on what was asked, not on the stale report.
    slider_volume: Optimistic<i32>,
    charging: bool,
    /// An input picked here, shown straight away until the receiver reports it (or gives up).
    pending_source: Optimistic<String>,
    /// The same for power: the receiver can take a minute to report that it has come on.
    pending_power: Optimistic<bool>,
    /// And for mute, so a second double tap undoes the first even before the receiver reports.
    pending_mute: Optimistic<bool>,
}
impl Session {
    fn entity(&self, id: &str) -> Option<&Entity> {
        self.entities.iter().find(|e| e.entity_id == id)
    }

    fn target(&self, t: Target) -> Option<&Entity> {
        let id = match t {
            Target::Screen => self.screen.as_deref(),
            Target::Volume => self.volume.as_deref(),
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

    /// The receiver's volume limit, from the Home Assistant sensor that reads it; full scale without one.
    fn volume_max(&self) -> i32 {
        let suffix = &settings::get().volume_limit_sensor;
        self.entities
            .iter()
            .find(|e| !suffix.is_empty() && e.entity_id.ends_with(suffix.as_str()))
            .and_then(|e| e.attributes.get(Attr::Value.name()))
            .and_then(|v| v.as_f64().or_else(|| v.as_str()?.trim().parse().ok()))
            .map(|v| (v.round() as i32).clamp(1, MAX_VOLUME))
            .unwrap_or(MAX_VOLUME)
    }

    /// The level the next slider step starts from.
    fn volume_base(&self) -> i32 {
        self.model().volume
    }

    /// What the Home Assistant helpers ask of the key backlight. Without the mode helper the
    /// light alone decides: on means its colour, off means follow the remote.
    fn keys(&self) -> backlight::Ha {
        use backlight::{Ha, Mode, Rgb};
        let cfg = settings::get();
        let ending = |suffix: &str| self.entities.iter().find(|e| !suffix.is_empty() && e.entity_id.ends_with(suffix));
        let colour = ending(&cfg.keys_light).filter(|l| l.state() == MediaState::On).map(|light| {
            let at = |a| number(Some(light), a);
            let (h, s, v) = (at(Attr::Hue), at(Attr::Saturation) / 255.0, at(Attr::Brightness) / 255.0);
            // HSV to RGB, with the light's brightness as the value.
            let f = |n: f64| {
                let k = (n + h / 60.0) % 6.0;
                ((v - v * s * k.min(4.0 - k).clamp(0.0, 1.0)) * 255.0).round() as u8
            };
            Rgb(f(5.0), f(3.0), f(1.0))
        });
        let mode = match text(ending(&cfg.keys_mode), Attr::CurrentOption).to_lowercase().as_str() {
            "off" => Mode::Off,
            "custom" => Mode::Custom,
            "" if colour.is_some() => Mode::Custom,
            _ => Mode::Follow,
        };
        Ha { mode, colour }
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
        let receiver = self.target(Target::Volume).filter(|_| self.volume != self.screen);
        let volume_from = receiver.or(screen);
        let state = screen.map_or(MediaState::Unknown, Entity::state);
        let paused = state != MediaState::Playing;
        let position = if paused { self.position } else { self.position + self.position_at.elapsed().as_secs_f64() };
        let link = |e: Option<&Entity>| self.integrations.get(e?.integration_id.as_deref()?).copied();
        let reported = text(receiver, Attr::Source).to_string();
        let switching = self.pending_source.pending(&reported);
        let source = self.pending_source.resolve(reported);
        let (status, warning) = net::health(net::Health {
            wifi_ssid: &self.wifi_ssid,
            home_assistant: link(receiver),
            receiver: receiver.map(Entity::state),
            kodi_link: link(screen),
            kodi: screen.is_some().then_some(state),
            on_kodi: source.is_empty() || source.eq_ignore_ascii_case("kodi"),
        });
        Model {
            keys: self.keys(),
            wifi_ssid: self.wifi_ssid.clone(),
            wifi: self.wifi.clone(),
            wifi_band: self.wifi_band.clone(),
            scanning: self.scanning,
            status,
            warning,
            demo_ha_down: false,
            volume: {
                // Until the receiver has caught up, its reports are old levels on the way.
                self.slider_volume.resolve(number(volume_from, Attr::Volume).round() as i32)
            },
            muted: {
                self.pending_mute.resolve(volume_from.and_then(|e| e.flag(Attr::Muted)).unwrap_or(false))
            },
            source,
            sources: list(receiver, Attr::SourceList).into_iter().filter(|s| !settings::get().hides(s)).collect(),
            switching,
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
            battery: self.battery,
            charging: self.charging,
            awake: self.awake,
            receiver_on: {
                // The AVR is the source of truth: anything short of on (standby, unreachable, not yet
                // known) is off.
                let reported = receiver.is_none_or(|r| !matches!(r.state(), MediaState::Off | MediaState::Standby) && !r.state().is_gone());
                self.pending_power.resolve(reported)
            },
            volume_max: self.volume_max(),
        }
    }
}
pub(crate) async fn run(weak: &slint::Weak<AppWindow>, actions: &mut mpsc::UnboundedReceiver<UiAction>) -> anyhow::Result<()> {
    let cfg = coreapi::Config::from_env()?;
    set_status(weak, false, format!("Connecting to {}…", cfg.url));

    let (core, mut events) = coreapi::connect(cfg).await?;
    let entities = core.get_entities().await?;
    tracing::info!(entities = entities.len(), "get_entities");

    let power = core.request("get_power_mode", json!({})).await.map(|r| r.data).unwrap_or(Value::Null);
    let charger = core.request("get_battery_charger", json!({})).await.map(|r| r.data).unwrap_or(Value::Null);
    tracing::info!(%power, %charger, "power");
    let battery = power.pointer("/battery/capacity").or(charger.get("capacity")).and_then(Value::as_i64).unwrap_or(100) as i32;
    let supply = |v: &Value| v.get("power_supply").or_else(|| v.pointer("/battery/power_supply")).and_then(Value::as_bool);
    let charging = on_charger(supply(&power).or_else(|| supply(&charger))).unwrap_or(false);
    // A desktop build looks at a remote that is usually asleep on its dock, so it only follows
    // the remote's power state when asked to.
    let follow_power = std::env::var("UC_MODEL").is_ok_and(|m| m == "UCR3") || std::env::var_os("UC_FOLLOW_POWER").is_some();
    // IDLE only dims the display; treating it as asleep would black out a screen still in use.
    let is_awake = |mode: Option<PowerMode>| !follow_power || !matches!(mode, Some(PowerMode::LowPower | PowerMode::Suspend));
    let screen = pick_entity("UC_SCREEN_ENTITY", &entities, |_| true);
    let mut session = Session {
        volume: pick_entity("UC_VOLUME_ENTITY", &entities, |e| e.has_feature("volume_up_down")),
        position: 0.0,
        position_at: Instant::now(),
        battery,
        poster_url: String::new(),
        wifi_ssid: String::new(),
        wifi: Vec::new(),
        wifi_band: String::new(),
        scanning: false,
        integrations: HashMap::new(),
        joining: None,
        subtitle_ids: Vec::new(),
        slider_volume: Optimistic::new(SLIDER_HOLD, SLIDER_SETTLE),
        charging,
        pending_source: Optimistic::new(Duration::ZERO, SWITCH_TIMEOUT),
        pending_power: Optimistic::new(Duration::ZERO, POWER_TIMEOUT),
        pending_mute: Optimistic::new(Duration::ZERO, SLIDER_SETTLE),
        title: StableTitle::default(),
        timing: false,
        awake: is_awake(power.get("mode").and_then(|m| serde_json::from_value(m.clone()).ok())),
        screen,
        entities,
    };
    load_settings(weak, &core).await;
    (session.wifi_ssid, session.wifi) = net::wifi(&core).await;
    session.wifi_band = wifi_band(&core).await;
    session.integrations = net::integrations(&core).await;
    session.settle_title();
    session.sync_poster(weak);
    session.position = number(session.target(Target::Screen), Attr::MediaPosition);
    tracing::info!(screen = ?session.screen, volume = ?session.volume, "targets");
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
            let (ssid, networks) = net::wifi(&core).await;
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
                commands.send(entity, "volume".into(), json!({ "volume": v }));
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
                        let _ = tx.send(Later::Integrations(net::integrations(&core).await));
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
                if session.joining.is_some_and(|(_, since)| since.elapsed() > JOIN_TIMEOUT) {
                    let _ = later_tx.send(Later::JoinDone(false));
                }
                session.settle_title();
                ask_title(&mut session, &later_tx);
                render(weak, &session.model());
            }
            Some(later) = later_rx.recv() => {
                match later {
                    Later::Wifi(ssid, networks) => {
                        (session.wifi_ssid, session.wifi, session.scanning) = (ssid, networks, false);
                    }
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
                        if session.joining.take().is_some() {
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
                        tracing::info!(mode = ?p.mode, "power mode");
                        session.awake = is_awake(Some(p.mode));
                        if p.mode == PowerMode::Normal {
                            keys.repaint();
                            // The core restores its own idea of the keys as it wakes; paint over it
                            // again once it has.
                            let keys = keys.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_millis(400)).await;
                                keys.repaint();
                            });
                        }
                        if let Some(on) = on_charger(p.on_charger()) {
                            session.charging = on;
                        }
                        render(weak, &session.model());
                    }
                    CoreEvent::BatteryStatus(b) => {
                        session.battery = b.capacity.into();
                        if let Some(on) = on_charger(b.power_supply) {
                            session.charging = on;
                        }
                        render(weak, &session.model());
                    }
                    CoreEvent::WifiChange(w) => {
                        if let Some((ours, _)) = session.joining {
                            // A NETWORK_NOT_FOUND without our id is only a scan notice, not the outcome.
                            let outcome = match w.event {
                                WifiEvent::Connected => Some(true),
                                WifiEvent::WrongKey => Some(false),
                                WifiEvent::NetworkNotFound if w.id.is_some() && w.id == ours => Some(false),
                                _ => None,
                            };
                            if let Some(ok) = outcome {
                                let _ = later_tx.send(Later::JoinDone(ok));
                            }
                        }
                        if matches!(w.event, WifiEvent::ScanCompleted | WifiEvent::Connected | WifiEvent::Disconnected) {
                            refresh_wifi(Duration::ZERO);
                        }
                    }
                    CoreEvent::EntityChange(change) => {
                        if session.volume.is_some() && session.volume == change.entity_id {
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
                // A Kodi action, kept for the poster wall's prediction; and whether this is a mute toggle, which
                // the receiver may not support as such.
                let mut pressed: Option<KodiAction> = None;
                let mut toggle_mute = false;
                let (target, cmd, params) = match action {
                    UiAction::Kodi(a) => {
                        pressed = Some(a);
                        (Target::Screen, a.command().to_string(), Value::Null)
                    }
                    UiAction::SelectSource(name) => {
                        session.pending_source.ask(name.clone());
                        render(weak, &session.model());
                        (Target::Volume, "select_source".to_string(), json!({ "source": name }))
                    }
                    UiAction::Power(what) => {
                        // Not wired yet: a stray tap in a prototype should not switch the remote off.
                        tracing::info!("power action {what} (not sent)");
                        continue;
                    }
                    UiAction::PowerHeld => {
                        let on = session.model().receiver_on;
                        session.pending_power.ask(!on);
                        render(weak, &session.model());
                        (Target::Volume, if on { "off" } else { "on" }.to_string(), Value::Null)
                    }
                    UiAction::Backlight(wanted) => {
                        keys.show(wanted);
                        continue;
                    }
                    UiAction::Display { brightness, auto } => {
                        let core = core.clone();
                        tokio::spawn(async move {
                            let cfg = json!({ "brightness": brightness, "auto_brightness": auto });
                            match core.request("set_display_cfg", cfg).await {
                                Ok(r) if r.is_ok() => {}
                                other => tracing::warn!("set_display_cfg: {other:?}"),
                            }
                        });
                        continue;
                    }
                    UiAction::Seek(by) => {
                        let m = session.model();
                        let to = (m.position + by as f64).clamp(0.0, m.duration.max(0.0));
                        (session.position, session.position_at) = (to, Instant::now());
                        render(weak, &session.model());
                        (Target::Screen, "seek".to_string(), json!({ "media_position": to.round() as i64 }))
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
                        (Target::Screen, "select_sound_mode".to_string(), json!({ "mode": mode }))
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
                        let v = slide(session.volume_base(), n, session.volume_max());
                        feedback::play(if v == session.volume_base() { feedback::SLIDER_LIMIT } else { feedback::VOLUME });
                        session.slider_volume.ask(v);
                        flash_volume(weak, &overlay);
                        render(weak, &session.model());
                        let send = volume_gate.offer(v, Instant::now());
                        pace(&mut volume_gate, send, &session);
                        continue;
                    }
                    UiAction::Slider(slider::Gesture::DoubleTap) => {
                        feedback::play(feedback::SLIDER_MUTE);
                        flash_volume(weak, &overlay);
                        toggle_mute = true;
                        (Target::Volume, ReceiverCmd::MuteToggle.command().to_string(), Value::Null)
                    }
                    UiAction::WifiBand => {
                        let Some(next) = (match session.wifi_band.as_str() { "a" => Some("b"), "b" => Some("a"), _ => None }) else { continue };
                        let cfg = json!({ "wifi": { "band": next } });
                        match core.request("set_network_cfg", cfg).await {
                            Ok(r) if r.is_ok() => {
                                tracing::info!("wifi band now {next}");
                                session.wifi_band = r.data.pointer("/wifi/band").and_then(Value::as_str).unwrap_or(next).to_string();
                            }
                            other => tracing::warn!("set_network_cfg band {next}: {other:?}"),
                        }
                        render(weak, &session.model());
                        // The remote reconnects on the new band; look again once it has.
                        refresh_wifi(Duration::from_secs(10));
                        continue;
                    }
                    UiAction::WifiScan => {
                        session.scanning = true;
                        render(weak, &session.model());
                        let core = core.clone();
                        tokio::spawn(async move {
                            let _ = core.request("wifi_scan_start", json!({})).await;
                        });
                        // SCAN_COMPLETED normally brings the results sooner; this covers a missed event.
                        refresh_wifi(Duration::from_secs(6));
                        continue;
                    }
                    UiAction::WifiJoin { ssid, hex, password } => {
                        let mut network = if hex.is_empty() { json!({ "ssid": ssid }) } else { json!({ "ssid_hex": hex }) };
                        if !password.is_empty() {
                            network["password"] = json!(password);
                        }
                        session.joining = Some((None, Instant::now()));
                        match core.request("add_wifi_network", network).await {
                            Ok(r) if r.is_ok() => {
                                session.joining = Some((r.data.get("id").and_then(Value::as_i64), Instant::now()));
                            }
                            other => {
                                tracing::warn!("joining {ssid} could not start: {other:?}");
                                let _ = later_tx.send(Later::JoinDone(false));
                            }
                        }
                        continue;
                    }
                    UiAction::Button(b) => match b.action() {
                        Action::Kodi(a) => {
                            pressed = Some(a);
                            (Target::Screen, a.command().to_string(), Value::Null)
                        }
                        Action::Receiver(r) => {
                            flash_volume(weak, &overlay);
                            match r {
                                ReceiverCmd::VolumeUp | ReceiverCmd::VolumeDown => feedback::play(feedback::VOLUME),
                                ReceiverCmd::MuteToggle => toggle_mute = true,
                            }
                            (Target::Volume, r.command().to_string(), Value::Null)
                        }
                    },
                    UiAction::DemoKey(_) => continue,
                };
                let Some(entity) = session.target(target) else {
                    tracing::warn!("{cmd}: no {target:?} entity");
                    continue;
                };
                // Not every player has a toggle; the Denon via Home Assistant only has mute and unmute.
                // The receiver reports late, so the toggle follows what this remote last asked for.
                let cmd = if toggle_mute {
                    let (muted, toggles) = (session.model().muted, entity.has_feature("mute_toggle"));
                    session.pending_mute.ask(!muted);
                    render(weak, &session.model());
                    if toggles { cmd } else { if muted { "unmute" } else { "mute" }.to_string() }
                } else {
                    cmd
                };
                let Some(entity) = session.target(target) else { continue };
                if let Some(a) = pressed {
                    library::predict(weak, a);
                }
                commands.send(entity, cmd, params);
                if target == Target::Screen {
                    library::poke();
                }
            }
        }
    }
    Ok(())
}
/// The menu's current values. Each is optional: a core that lacks a message just leaves a blank.
pub(crate) async fn load_settings(weak: &slint::Weak<AppWindow>, core: &coreapi::Core) {
    let (display, haptic, power, sound) =
        tokio::join!(core.fetch("get_display_cfg"), core.fetch("get_haptic_cfg"), core.fetch("get_power_saving_cfg"), core.fetch("get_sound_cfg"));
    feedback::set_core_config(
        haptic.as_ref().and_then(|v| v.get("enabled")).and_then(Value::as_bool),
        sound.as_ref().and_then(|v| v.get("enabled")).and_then(Value::as_bool),
        sound.as_ref().and_then(|v| v.get("volume")).and_then(Value::as_u64).map(|v| v.min(100) as u8),
    );
    let secs = |v: &Option<Value>, key: &str| v.as_ref().and_then(|v| v.get(key)).and_then(Value::as_i64).map(duration_label);
    let brightness = display.as_ref().and_then(|v| v.get("brightness")).and_then(Value::as_i64);
    let auto = display.as_ref().and_then(|v| v.get("auto_brightness")).and_then(Value::as_bool);
    let haptics = haptic.as_ref().and_then(|v| v.get("enabled")).and_then(Value::as_bool);
    let (screen_off, sleep) = (secs(&power, "display_off_sec"), secs(&power, "standby_sec"));
    ui(weak, move |app| {
        if let Some(b) = brightness {
            app.set_brightness(b as i32);
        }
        if let Some(a) = auto {
            app.set_auto_brightness(a);
        }
        app.set_haptics(haptics.map_or("", |on| if on { "On" } else { "Off" }).into());
        app.set_screen_off_after(screen_off.unwrap_or_default().into());
        app.set_sleep_after(sleep.unwrap_or_default().into());
    });
}
pub(crate) type Queued = (String, Value, Instant);
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

    fn send(&self, entity: &Entity, cmd: String, params: Value) {
        let id = entity.entity_id.clone();
        let mut queues = self.queues.borrow_mut();
        let tx = queues.entry(id.clone()).or_insert_with(|| {
            let (tx, mut rx) = mpsc::unbounded_channel::<Queued>();
            let (core, name) = (self.core.clone(), entity.name().to_string());
            tokio::spawn(async move {
                while let Some((cmd, params, made)) = rx.recv().await {
                    let waited = made.elapsed();
                    let sent = Instant::now();
                    let data = json!({ "entity_id": id, "cmd_id": cmd, "params": params });
                    let mut answer = core.submit("execute_entity_command", data).await;
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
                        tracing::info!("{name} · {cmd} — {outcome} in {} ms (queued {} ms)", sent.elapsed().as_millis(), waited.as_millis());
                    });
                }
            });
            tx
        });
        let _ = tx.send((cmd, params, Instant::now()));
    }
}

/// Whether the remote sits on its charger, as reported. Previews of the simulator (always on its charger) can
/// pretend it is on battery.
fn on_charger(reported: Option<bool>) -> Option<bool> {
    if cfg!(feature = "preview") && std::env::var_os("UC_PREVIEW_ON_BATTERY").is_some() {
        return Some(false);
    }
    reported
}
