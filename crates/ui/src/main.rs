//! remote-ui: a single-purpose theater remote. The receiver's volume and input come from one
//! entity, Kodi's playback from another; the screen shows whichever the receiver is switched to.
//! The tokio runtime runs on its own thread and pushes a `Model` into the Slint event loop via
//! `invoke_from_event_loop`; UI callbacks flow back over a channel. See docs/plan.md.

mod backlight;
mod device;
#[cfg(feature = "device")]
mod evdev_input;
mod haptics;
mod input;
mod keyboard;
mod net;
mod settings;
mod shots;

use input::{Button, Target};
use serde_json::{json, Value};
use slint::{ComponentHandle, Model as _, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing_subscriber::EnvFilter;

slint::include_modules!();

const RECONNECT_DELAY: Duration = Duration::from_secs(2);
const VOLUME_OVERLAY: Duration = Duration::from_millis(1800);
const JOIN_TIMEOUT: Duration = Duration::from_secs(45);

pub enum UiAction {
    Command(String),
    Key(String),
    SelectSource(String),
    Power(String),
    PowerHeld,
    WifiScan,
    WifiJoin { ssid: String, hex: String, password: String },
    /// Light these key zones in this colour.
    Backlight(Vec<u8>, [u8; 3]),
}

/// Results of work that finishes later than the request that started it.
enum Later {
    Wifi(String, Vec<net::Network>),
    Integrations(HashMap<String, bool>),
    JoinDone(bool),
}

/// Everything the screen shows, independent of where it came from (core or demo).
#[derive(Clone, Debug, Default)]
pub struct Model {
    pub volume: i32,
    pub muted: bool,
    pub source: String,
    pub sources: Vec<String>,
    pub sound_mode: String,
    pub title: String,
    pub paused: bool,
    pub duration: f64,
    pub position: f64,
    pub media_format: String,
    pub battery: i32,
    pub awake: bool,
    pub receiver_on: bool,
    pub wifi_ssid: String,
    pub wifi: Vec<net::Network>,
    pub scanning: bool,
    pub status: Vec<net::StatusLine>,
    pub warning: String,
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
            sound_mode: "DOLBY DIGITAL".into(),
            title: "Blade Runner 2049".into(),
            paused: false,
            duration: 9840.0,
            position: 4328.0,
            media_format: "DTS HD MA 7.1".into(),
            battery: 50,
            awake: true,
            receiver_on: true,
            wifi_ssid: "HomeNetwork".into(),
            wifi: net::demo_networks("HomeNetwork"),
            ..Default::default()
        }
        .with_demo_health()
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
    fn on_kodi(&self) -> bool {
        self.source.is_empty() || self.source.eq_ignore_ascii_case("kodi")
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_ansi(cfg!(not(feature = "device")))
        .init();

    #[cfg(feature = "device")]
    select_device_renderer();
    let app = AppWindow::new()?;
    #[cfg(feature = "device")]
    evdev_input::start(&app);
    let accent = std::env::var("UC_ACCENT").ok().or_else(|| settings::get().accent.clone());
    if let Some(accent) = accent.as_deref().and_then(parse_color) {
        set_accent(&app, accent);
    }

    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1));
    let frame = arg("--frame").map(|p| device::Device::load(p.as_ref())).transpose()?;
    let (act_tx, mut act_rx) = mpsc::unbounded_channel();
    wire_keyboard(&app, act_tx.clone());
    if let Some(dir) = arg("--shots") {
        return shots::run(app, dir.into(), frame);
    }
    let record = arg("--record");
    let demo = record.is_some() || args.iter().any(|a| a == "--demo") || std::env::var_os("UC_DEMO").is_some();
    if demo {
        apply_demo_settings(&app);
    }

    let weak = app.as_weak();
    let send = |f: fn(String) -> UiAction| {
        let tx = act_tx.clone();
        move |s: SharedString| {
            let _ = tx.send(f(s.into()));
        }
    };
    app.on_command(send(UiAction::Command));
    app.on_key(send(UiAction::Key));
    app.on_select_source(send(UiAction::SelectSource));
    app.on_power_action(send(UiAction::Power));
    app.on_haptic(|name| {
        if let Some(cue) = haptics::Cue::from_ui(&name) {
            haptics::play(cue);
        }
    });
    app.on_wifi_scan({
        let tx = act_tx.clone();
        move || {
            let _ = tx.send(UiAction::WifiScan);
        }
    });
    app.on_wifi_join({
        let tx = act_tx.clone();
        move |ssid, hex, password| {
            let _ = tx.send(UiAction::WifiJoin { ssid: ssid.into(), hex: hex.into(), password: password.into() });
        }
    });
    app.on_power_held({
        let tx = act_tx.clone();
        move || {
            let _ = tx.send(UiAction::PowerHeld);
        }
    });

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async move {
            if demo {
                run_demo(&weak, &mut act_rx).await;
                return;
            }
            loop {
                match run(&weak, &mut act_rx).await {
                    Ok(()) => set_status(&weak, false, "Disconnected, retrying…".to_string()),
                    Err(e) => {
                        tracing::error!("core session failed: {e:#}");
                        set_status(&weak, false, format!("{e:#}"));
                    }
                }
                tokio::time::sleep(RECONNECT_DELAY).await;
            }
        });
    });

    if let Some(page) = arg("--page").and_then(|p| p.parse().ok()) {
        app.invoke_go(page, 0);
    }
    let _snap = arg("--snap").map(|out| shots::snap(&app, out.into(), device::Device::load_opt(arg("--frame"))));
    // The window is the source of truth for what is on screen, so the key backlight follows it
    // from here; the core is only told when the answer changes.
    let _backlight = {
        let (weak, tx) = (app.as_weak(), act_tx.clone());
        let mut last = (Vec::new(), [0u8; 3]);
        let timer = slint::Timer::default();
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(150), move || {
            let Some(app) = weak.upgrade() else { return };
            let (state, accent) = backlight::observe(&app);
            let now = (backlight::zones(state).to_vec(), backlight::colour(state, accent));
            if now != last {
                last = now.clone();
                let _ = tx.send(UiAction::Backlight(now.0, now.1));
            }
        });
        timer
    };
    let _recording = match record {
        Some(out) => {
            let device = frame.ok_or_else(|| anyhow::anyhow!("--record needs --frame <remote photo>"))?;
            Some(shots::record(&app, act_tx.clone(), device, out.into(), arg("--script").map(String::as_str))?)
        }
        None => None,
    };

    app.run()?;
    Ok(())
}

/// GLES2 first; if the GPU path cannot start, draw in software so there is still a picture.
/// `"renderer": "software"` in settings.json skips straight to the fallback.
#[cfg(feature = "device")]
fn select_device_renderer() {
    let wanted = settings::get().renderer.as_deref().unwrap_or("femtovg");
    for renderer in [wanted, "software"] {
        let selected = slint::BackendSelector::new().backend_name("linuxkms".into()).renderer_name(renderer.into()).select();
        match selected {
            Ok(()) => {
                tracing::info!("display: linuxkms with the {renderer} renderer");
                return;
            }
            Err(e) => tracing::warn!("linuxkms with {renderer} failed: {e}"),
        }
    }
}

/// The keyboard lives on the UI thread: every d-pad press must redraw at once, with no round trip.
fn wire_keyboard(app: &AppWindow, tx: mpsc::UnboundedSender<UiAction>) {
    let kb = Rc::new(RefCell::new(keyboard::Keyboard::new()));
    let weak = app.as_weak();
    let after = Rc::new({
        let kb = kb.clone();
        move |outcome: keyboard::Outcome| {
            let Some(app) = weak.upgrade() else { return };
            let icons = app.global::<Icons>();
            let cells: Vec<KeyCell> = kb
                .borrow()
                .cells()
                .into_iter()
                .map(|c| KeyCell {
                    label: c.label.into(),
                    icon: match c.kind {
                        keyboard::Kind::Shift => icons.get_shift(),
                        keyboard::Kind::Delete => icons.get_delete(),
                        _ => SharedString::new(),
                    },
                    x: c.x,
                    y: c.y,
                    w: c.w,
                    h: c.h,
                    focused: c.focused,
                    style: match c.style {
                        keyboard::Style::Char => 0,
                        keyboard::Style::Action => 1,
                        keyboard::Style::ActionOn => 2,
                        keyboard::Style::JoinDim => 3,
                        keyboard::Style::JoinReady => 4,
                    },
                })
                .collect();
            app.set_kb_keys(ModelRc::new(VecModel::from(cells)));
            app.set_kb_text(kb.borrow().text().into());
            match outcome {
                keyboard::Outcome::Stay => {}
                keyboard::Outcome::Leave => app.invoke_back(),
                keyboard::Outcome::Join(password) => {
                    let target = app.get_wifi_target();
                    app.set_join_state(1);
                    let _ = tx.send(UiAction::WifiJoin { ssid: target.ssid.into(), hex: target.ssid_hex.into(), password });
                }
            }
        }
    });
    after(keyboard::Outcome::Stay);
    app.on_kb_reset({
        let (kb, after) = (kb.clone(), after.clone());
        move || {
            kb.borrow_mut().reset();
            after(keyboard::Outcome::Stay);
        }
    });
    app.on_kb_nav({
        let (kb, after) = (kb.clone(), after.clone());
        move |name| {
            let outcome = kb.borrow_mut().key(&name);
            after(outcome);
        }
    });
    app.on_kb_tap(move |index| {
        let outcome = kb.borrow_mut().tap(index.max(0) as usize);
        after(outcome);
    });
}

/// The interface colour. Callable at any time from the UI thread.
pub fn set_accent(app: &AppWindow, color: slint::Color) {
    app.global::<Theme>().set_accent(color);
}

/// `#RRGGBB` or `RRGGBB`.
pub fn parse_color(s: &str) -> Option<slint::Color> {
    let hex = s.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(hex, 16).ok()?;
    Some(slint::Color::from_rgb_u8((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

pub fn apply_demo_settings(app: &AppWindow) {
    app.set_haptics("On".into());
    app.set_screen_off_after("60 s".into());
    app.set_sleep_after("3 min".into());
}

struct Session {
    entities: Vec<Value>,
    screen: Option<String>,
    volume: Option<String>,
    position: f64,
    position_at: Instant,
    battery: i32,
    poster_url: String,
    awake: bool,
    wifi_ssid: String,
    wifi: Vec<net::Network>,
    scanning: bool,
    integrations: HashMap<String, bool>,
    /// The network id of a join in progress, and when it started.
    joining: Option<(Option<i64>, Instant)>,
}

impl Session {
    fn entity(&self, id: &str) -> Option<&Value> {
        self.entities.iter().find(|e| entity_id(e) == id)
    }

    fn target(&self, t: Target) -> Option<&Value> {
        let id = match t {
            Target::Screen => self.screen.as_deref(),
            Target::Volume => self.volume.as_deref(),
        };
        id.and_then(|id| self.entity(id))
    }

    /// Merge an `entity_change` into the cached entity; returns whether it applied.
    fn apply_change(&mut self, data: &Value) -> bool {
        let Some(id) = data.get("entity_id").and_then(Value::as_str) else { return false };
        let Some(attrs) = data.pointer("/new_state/attributes").and_then(Value::as_object) else { return false };
        let Some(entity) = self.entities.iter_mut().find(|e| entity_id(e) == id) else { return false };
        let Some(obj) = entity.as_object_mut() else { return false };
        let cached = obj.entry("attributes").or_insert_with(|| Value::Object(Default::default()));
        for (k, v) in attrs {
            cached[k.as_str()] = v.clone();
        }
        if self.screen.as_deref() == Some(id) {
            if let Some(p) = attrs.get("media_position").and_then(Value::as_f64) {
                self.position = p;
                self.position_at = Instant::now();
            }
        }
        true
    }

    /// Fetch the artwork whenever Kodi reports a different image URL.
    fn sync_poster(&mut self, weak: &slint::Weak<AppWindow>) {
        let url = attr_str(self.target(Target::Screen), "media_image_url");
        if url == self.poster_url {
            return;
        }
        self.poster_url = url.clone();
        let weak = weak.clone();
        tokio::task::spawn_blocking(move || {
            let poster = if url.is_empty() { None } else { fetch_poster(&url).map_err(|e| tracing::warn!("poster {url}: {e:#}")).ok() };
            ui(&weak, move |app| {
                app.set_poster(match poster {
                    Some(img) => slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(img.as_raw(), img.width(), img.height())),
                    None => slint::Image::default(),
                });
            });
        });
    }

    fn model(&self) -> Model {
        let screen = self.target(Target::Screen);
        let receiver = self.target(Target::Volume).filter(|_| self.volume != self.screen);
        let volume_from = receiver.or(screen);
        let state = attr_str(screen, "state");
        let paused = state != "PLAYING";
        let position = if paused { self.position } else { self.position + self.position_at.elapsed().as_secs_f64() };
        let link = |e: Option<&Value>| {
            let id = e?.get("integration_id")?.as_str()?;
            self.integrations.get(id).copied()
        };
        let source = attr_str(receiver, "source");
        let receiver_state = receiver.map(|r| attr_str(Some(r), "state"));
        let (status, warning) = net::health(net::Health {
            wifi_ssid: &self.wifi_ssid,
            home_assistant: link(receiver),
            receiver: receiver_state.as_deref(),
            kodi_link: link(screen),
            kodi: screen.is_some().then_some(state.as_str()),
            on_kodi: source.is_empty() || source.eq_ignore_ascii_case("kodi"),
        });
        Model {
            wifi_ssid: self.wifi_ssid.clone(),
            wifi: self.wifi.clone(),
            scanning: self.scanning,
            status,
            warning,
            demo_ha_down: false,
            volume: attr_f64(volume_from, "volume").round() as i32,
            muted: volume_from.and_then(|e| e.pointer("/attributes/muted")).and_then(Value::as_bool).unwrap_or(false),
            source,
            sources: receiver
                .and_then(|e| e.pointer("/attributes/source_list"))
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).filter(|s| !settings::get().hides(s)).map(String::from).collect())
                .unwrap_or_default(),
            sound_mode: attr_str(receiver, "sound_mode"),
            title: attr_str(screen, "media_title"),
            paused,
            duration: attr_f64(screen, "media_duration"),
            position,
            media_format: String::new(),
            battery: self.battery,
            awake: self.awake,
            receiver_on: receiver.is_none_or(|r| attr_str(Some(r), "state") != "OFF"),
        }
    }
}

async fn run(weak: &slint::Weak<AppWindow>, actions: &mut mpsc::UnboundedReceiver<UiAction>) -> anyhow::Result<()> {
    let cfg = coreapi::Config::from_env()?;
    set_status(weak, false, format!("Connecting to {}…", cfg.url));

    let (core, mut events) = coreapi::connect(cfg).await?;
    let entities = core.get_entities().await?;
    tracing::info!(entities = entities.len(), "get_entities");

    let power = core.request("get_power_mode", json!({})).await.map(|r| r.data).unwrap_or(Value::Null);
    let battery = power.pointer("/battery/capacity").and_then(Value::as_i64).unwrap_or(100) as i32;
    // A desktop build looks at a remote that is usually asleep on its dock, so it only follows
    // the remote's power state when asked to.
    let follow_power = std::env::var("UC_MODEL").is_ok_and(|m| m == "UCR3") || std::env::var_os("UC_FOLLOW_POWER").is_some();
    // IDLE only dims the display; treating it as asleep would black out a screen still in use.
    let is_awake = |mode: Option<&str>| !follow_power || !matches!(mode, Some("LOW_POWER" | "SUSPEND"));
    let screen = pick_entity("UC_SCREEN_ENTITY", &entities, |_| true);
    let mut session = Session {
        volume: pick_entity("UC_VOLUME_ENTITY", &entities, |e| has_feature(e, "volume_up_down")),
        position: 0.0,
        position_at: Instant::now(),
        battery,
        poster_url: String::new(),
        wifi_ssid: String::new(),
        wifi: Vec::new(),
        scanning: false,
        integrations: HashMap::new(),
        joining: None,
        awake: is_awake(power.get("mode").and_then(Value::as_str)),
        screen,
        entities,
    };
    load_settings(weak, &core).await;
    (session.wifi_ssid, session.wifi) = net::wifi(&core).await;
    session.integrations = net::integrations(&core).await;
    session.sync_poster(weak);
    session.position = attr_f64(session.target(Target::Screen), "media_position");
    tracing::info!(screen = ?session.screen, volume = ?session.volume, "targets");

    render(weak, &session.model());
    set_status(weak, true, String::new());

    // Presses made while offline would otherwise fire in a burst on reconnect.
    while actions.try_recv().is_ok() {}

    let overlay = Arc::new(AtomicU64::new(0));
    let (later_tx, mut later_rx) = mpsc::unbounded_channel::<Later>();
    let (backlight_tx, backlight_rx) = tokio::sync::watch::channel((Vec::<u8>::new(), [0u8; 3]));
    tokio::spawn(drive_backlight(core.clone(), backlight_rx));
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
                if session.joining.is_some_and(|(_, since)| since.elapsed() > JOIN_TIMEOUT) {
                    let _ = later_tx.send(Later::JoinDone(false));
                }
                render(weak, &session.model());
            }
            Some(later) = later_rx.recv() => {
                match later {
                    Later::Wifi(ssid, networks) => {
                        (session.wifi_ssid, session.wifi, session.scanning) = (ssid, networks, false);
                    }
                    Later::Integrations(map) => session.integrations = map,
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
                if ev.msg == "power_mode_change" {
                    session.awake = is_awake(ev.data.get("mode").and_then(Value::as_str));
                    render(weak, &session.model());
                }
                if ev.msg == "wifi_change" {
                    let event = ev.data.get("event").and_then(Value::as_str).unwrap_or_default();
                    let id = ev.data.get("id").and_then(Value::as_i64);
                    if let Some((ours, _)) = session.joining {
                        // A NETWORK_NOT_FOUND without our id is only a scan notice, not the outcome.
                        let outcome = match event {
                            "CONNECTED" => Some(true),
                            "WRONG_KEY" => Some(false),
                            "NETWORK_NOT_FOUND" if id.is_some() && id == ours => Some(false),
                            _ => None,
                        };
                        if let Some(ok) = outcome {
                            let _ = later_tx.send(Later::JoinDone(ok));
                        }
                    }
                    if matches!(event, "SCAN_COMPLETED" | "CONNECTED" | "DISCONNECTED") {
                        refresh_wifi(Duration::ZERO);
                    }
                }
                if ev.msg == "entity_change" && session.apply_change(&ev.data) {
                    session.sync_poster(weak);
                    render(weak, &session.model());
                }
            }
            Some(action) = actions.recv() => {
                let (target, cmd, params) = match action {
                    UiAction::Command(cmd) => (Target::Screen, cmd, Value::Null),
                    UiAction::SelectSource(name) => (Target::Volume, "select_source".to_string(), json!({ "source": name })),
                    UiAction::Power(what) => {
                        // Not wired yet: a stray tap in a prototype should not switch the remote off.
                        tracing::info!("power action {what} (not sent)");
                        continue;
                    }
                    UiAction::PowerHeld => {
                        let on = session.model().receiver_on;
                        (Target::Volume, if on { "off" } else { "on" }.to_string(), Value::Null)
                    }
                    UiAction::Backlight(zones, rgb) => {
                        let _ = backlight_tx.send((zones, rgb));
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
                            Ok(r) if (200..300).contains(&r.code) => {
                                session.joining = Some((r.data.get("id").and_then(Value::as_i64), Instant::now()));
                            }
                            other => {
                                tracing::warn!("joining {ssid} could not start: {other:?}");
                                let _ = later_tx.send(Later::JoinDone(false));
                            }
                        }
                        continue;
                    }
                    UiAction::Key(text) => match Button::from_key(&text) {
                        Some(b) => {
                            let (target, cmd) = b.route();
                            if target == Target::Volume {
                                flash_volume(weak, &overlay);
                            }
                            (target, cmd.to_string(), Value::Null)
                        }
                        None => continue,
                    },
                };
                let Some(entity) = session.target(target) else {
                    tracing::warn!("{cmd}: no {target:?} entity");
                    continue;
                };
                // Not every player has a toggle; the Denon via Home Assistant only has mute and unmute.
                let cmd = if cmd == "mute_toggle" && !has_feature(entity, "mute_toggle") {
                    let muted = entity.pointer("/attributes/muted").and_then(Value::as_bool).unwrap_or(false);
                    if muted { "unmute" } else { "mute" }.to_string()
                } else {
                    cmd
                };
                send_command(&core, entity, cmd, params);
            }
        }
    }
    Ok(())
}

/// Keeps the key backlight in step with the latest request. The core only changes the zones named
/// in an update and leaves the rest showing what they had, so each change is two updates: black
/// for the zones that should be off, then the colour for the ones that should be on. Running them
/// from one task keeps the pairs in order, and a newer request replaces one not yet sent.
async fn drive_backlight(core: coreapi::Core, mut wanted: tokio::sync::watch::Receiver<(Vec<u8>, [u8; 3])>) {
    let names = |zones: &[u8]| zones.iter().map(|z| format!("ZONE_{z}")).collect::<Vec<_>>();
    while wanted.changed().await.is_ok() {
        let (lit, rgb) = wanted.borrow_and_update().clone();
        let dark: Vec<u8> = (1..=backlight::ZONE_COUNT).filter(|z| !lit.contains(z)).collect();
        for (zones, rgb) in [(dark, [0, 0, 0]), (lit, rgb)] {
            if zones.is_empty() {
                continue;
            }
            let cfg = json!({ "static_color": { "rgb": rgb, "zones": names(&zones) } });
            match core.request("set_button_cfg", cfg).await {
                Ok(r) if (200..300).contains(&r.code) => {}
                other => tracing::debug!("backlight: {other:?}"),
            }
        }
    }
}

/// Leave the keyboard for the network list, or for the failure page.
fn finish_join(weak: &slint::Weak<AppWindow>, ok: bool) {
    ui(weak, move |app| {
        if app.get_page() != 6 {
            return;
        }
        app.set_join_state(if ok { 0 } else { 2 });
        app.invoke_go(if ok { 5 } else { 8 }, 0);
    });
}

/// Plain HTTP only: Kodi serves its artwork from its own web server on the LAN.
fn fetch_poster(url: &str) -> anyhow::Result<image::RgbaImage> {
    use std::io::Read;
    let mut bytes = Vec::new();
    ureq::get(url).timeout(Duration::from_secs(8)).call()?.into_reader().take(16 << 20).read_to_end(&mut bytes)?;
    let img = image::load_from_memory(&bytes)?;
    Ok(img.resize_to_fill(432, 648, image::imageops::FilterType::Triangle).to_rgba8())
}

/// The menu's current values. Each is optional: a core that lacks a message just leaves a blank.
async fn load_settings(weak: &slint::Weak<AppWindow>, core: &coreapi::Core) {
    let get = |msg: &'static str| async move {
        core.request(msg, json!({})).await.ok().filter(|r| (200..300).contains(&r.code)).map(|r| r.data)
    };
    let (display, haptic, power) = tokio::join!(get("get_display_cfg"), get("get_haptic_cfg"), get("get_power_saving_cfg"));
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

fn duration_label(secs: i64) -> String {
    if secs >= 60 && secs % 60 == 0 {
        format!("{} min", secs / 60)
    } else {
        format!("{secs} s")
    }
}

/// No core at all: the keys and touch drive a local model, so the UI can be explored offline.
async fn run_demo(weak: &slint::Weak<AppWindow>, actions: &mut mpsc::UnboundedReceiver<UiAction>) {
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
                Later::Integrations(_) => {}
                Later::JoinDone(ok) => {
                    if ok {
                        m.wifi_ssid = std::mem::take(&mut joining);
                        m.wifi = net::demo_networks(&m.wifi_ssid);
                        m.wifi.sort_by_key(|n| !n.connected);
                    }
                    finish_join(weak, ok);
                }
            },
            Some(action) = actions.recv() => match action {
                UiAction::WifiScan => {
                    m.scanning = true;
                    let mut found = net::demo_networks(&m.wifi_ssid);
                    found.sort_by_key(|n| !n.connected);
                    after(1400, Later::Wifi(m.wifi_ssid.clone(), found));
                }
                UiAction::WifiJoin { ssid, .. } => {
                    let ok = tried == ssid;
                    tried = ssid.clone();
                    joining = ssid;
                    after(1800, Later::JoinDone(ok));
                }
                UiAction::Backlight(..) => continue,
                UiAction::Key(text) if text == "x" => m.demo_ha_down = !m.demo_ha_down,
                UiAction::SelectSource(name) => m.source = name,
                UiAction::Command(cmd) => tracing::info!("kodi command {cmd}"),
                UiAction::Power(what) => tracing::info!("power action {what}"),
                UiAction::PowerHeld => m.receiver_on = !m.receiver_on,
                // Stand-in for the core's power-mode event.
                UiAction::Key(text) if text == "z" => m.awake = !m.awake,
                UiAction::Key(text) => match Button::from_key(&text) {
                    Some(Button::VolumeUp) => { m.volume = (m.volume + 1).min(98); m.muted = false; flash_volume(weak, &overlay); }
                    Some(Button::VolumeDown) => { m.volume = (m.volume - 1).max(0); m.muted = false; flash_volume(weak, &overlay); }
                    Some(Button::Mute) => { m.muted = !m.muted; flash_volume(weak, &overlay); }
                    Some(Button::Play) if m.title.is_empty() => m.title = title.clone(),
                    Some(Button::Play) => m.paused = !m.paused,
                    Some(Button::Stop) => m.title.clear(),
                    Some(b) => tracing::info!("key {}", b.name()),
                    None => {}
                },
            },
        }
        m = m.with_demo_health();
        render(weak, &m);
    }
}

/// Show the volume overlay and hide it again once the keys have been quiet for a moment.
fn flash_volume(weak: &slint::Weak<AppWindow>, generation: &Arc<AtomicU64>) {
    let mine = generation.fetch_add(1, Ordering::SeqCst) + 1;
    ui(weak, |app| app.set_volume_overlay(true));
    let (weak, generation) = (weak.clone(), generation.clone());
    tokio::spawn(async move {
        tokio::time::sleep(VOLUME_OVERLAY).await;
        if generation.load(Ordering::SeqCst) == mine {
            ui(&weak, |app| app.set_volume_overlay(false));
        }
    });
}

fn send_command(core: &coreapi::Core, entity: &Value, cmd: String, params: Value) {
    let id = entity_id(entity).to_string();
    let line = format!("{} · {cmd}", entity_name(entity));
    let core = core.clone();
    tokio::spawn(async move {
        let outcome = match core.execute_entity_command(&id, &cmd, params).await {
            Ok(r) if (200..300).contains(&r.code) => "ok".to_string(),
            Ok(r) => r.data.get("message").and_then(Value::as_str).unwrap_or("failed").to_string(),
            Err(e) => format!("{e:#}"),
        };
        tracing::info!("{line} — {outcome}");
    });
}

/// Env override first; otherwise prefer a Kodi media_player, then any matching media_player.
fn pick_entity(env: &str, entities: &[Value], accept: impl Fn(&Value) -> bool) -> Option<String> {
    if let Ok(id) = std::env::var(env) {
        return Some(id);
    }
    let players = || {
        entities
            .iter()
            .filter(|e| e.get("entity_type").and_then(Value::as_str) == Some("media_player"))
            .filter(|e| accept(e))
    };
    let is_kodi = |e: &&Value| {
        entity_id(e).to_lowercase().contains("kodi") || entity_name(e).to_lowercase().contains("kodi")
    };
    let pick = if env == "UC_SCREEN_ENTITY" {
        players().find(is_kodi)
    } else {
        // The receiver is the player that can switch inputs.
        players().find(|e| has_feature(e, "select_source") && !is_kodi(e))
    };
    pick.or_else(|| players().next()).map(|e| entity_id(e).to_string())
}

fn has_feature(e: &Value, feature: &str) -> bool {
    e.get("features")
        .and_then(Value::as_array)
        .is_some_and(|f| f.iter().any(|v| v.as_str() == Some(feature)))
}

fn ui(weak: &slint::Weak<AppWindow>, f: impl FnOnce(AppWindow) + Send + 'static) {
    let weak = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(app) = weak.upgrade() {
            f(app);
        }
    });
}

fn set_status(weak: &slint::Weak<AppWindow>, connected: bool, s: String) {
    ui(weak, move |app| {
        app.set_connected(connected);
        app.set_status(s.into());
    });
}

fn render(weak: &slint::Weak<AppWindow>, m: &Model) {
    let m = m.clone();
    ui(weak, move |app| apply(&app, &m));
}

/// Push a model into the window. UI thread only.
pub fn apply(app: &AppWindow, m: &Model) {
    let now = chrono::Local::now();
    let logos = app.global::<Logos>();

    app.set_clock(now.format("%H:%M").to_string().into());
    app.set_awake(m.awake);
    app.set_receiver_on(m.receiver_on);
    app.set_battery(m.battery);
    app.set_volume(m.volume);
    app.set_volume_db(volume_db(m.volume).into());
    app.set_muted(m.muted);

    app.set_source(m.source.as_str().into());
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
    let (brand, label) = format_badge(&m.sound_mode);
    app.set_format_brand(brand.into());
    app.set_format_label(label.into());

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
    let status: Vec<StatusRow> =
        m.status.iter().map(|s| StatusRow { label: s.label.into(), value: (&s.value).into(), level: s.level }).collect();
    if app.get_status_rows().iter().ne(status.iter().cloned()) {
        app.set_status_rows(ModelRc::new(VecModel::from(status)));
    }
    app.set_home_warning(m.warning.as_str().into());

    app.set_kodi(m.on_kodi());
    app.set_media_title(m.title.as_str().into());
    let left = (m.duration - m.position).max(0.0);
    app.set_progress(if m.duration > 0.0 { (m.position / m.duration) as f32 } else { 0.0 });
    app.set_time_left(if m.duration > 0.0 { time_left(left).into() } else { SharedString::new() });
    app.set_ends_at(if m.paused {
        "paused".into()
    } else if m.duration > 0.0 {
        let end = now + chrono::Duration::seconds(left as i64);
        end.format("ends %H:%M").to_string().into()
    } else {
        SharedString::new()
    });
    let (brand, label) = format_badge(&m.media_format);
    app.set_media_format_brand(brand.into());
    app.set_media_format_label(label.into());
}

/// Denon reports volume as dB + 80, so the receiver's own dB reading is the level minus 80.
fn volume_db(volume: i32) -> String {
    match volume - 80 {
        db if db < 0 => format!("−{} dB", -db),
        0 => "0 dB".to_string(),
        db => format!("+{db} dB"),
    }
}

fn time_left(secs: f64) -> String {
    let mins = (secs / 60.0).round() as i64;
    if mins >= 60 {
        format!("{}:{:02} left", mins / 60, mins % 60)
    } else {
        format!("{mins} min left")
    }
}

/// "DOLBY DIGITAL" → ("dolby", "Digital"): the logo carries the brand, the label the rest.
fn format_badge(mode: &str) -> (&'static str, String) {
    let upper = mode.trim().to_uppercase();
    let (brand, rest) = if let Some(r) = upper.strip_prefix("DOLBY") {
        ("dolby", r)
    } else if let Some(r) = upper.strip_prefix("DTS") {
        ("dts", r)
    } else {
        ("", upper.as_str())
    };
    let rest = rest.trim_matches(|c: char| c == ' ' || c == '-' || c == ':');
    let label = rest
        .split(' ')
        .map(|w| {
            if w.len() <= 3 || w.chars().any(|c| c.is_ascii_digit()) {
                w.to_string()
            } else {
                let mut c = w.chars();
                c.next().map(|f| f.to_string() + &c.as_str().to_lowercase()).unwrap_or_default()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    (brand, label)
}

fn source_logo(logos: &Logos, name: &str) -> SharedString {
    let n = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    if has(&["playstation", "ps4", "ps5"]) {
        logos.get_playstation()
    } else if has(&["xbox"]) {
        logos.get_xbox()
    } else if has(&["kodi"]) {
        logos.get_kodi()
    } else if has(&["android", "shield", "google"]) {
        logos.get_android()
    } else if has(&["bluetooth"]) {
        logos.get_bluetooth()
    } else if has(&["heos", "music", "tuner", "phono", "cd"]) {
        logos.get_music()
    } else {
        logos.get_generic()
    }
}

fn attr_str(e: Option<&Value>, key: &str) -> String {
    e.and_then(|e| e.pointer(&format!("/attributes/{key}")))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn attr_f64(e: Option<&Value>, key: &str) -> f64 {
    e.and_then(|e| e.pointer(&format!("/attributes/{key}"))).and_then(Value::as_f64).unwrap_or(0.0)
}

fn entity_id(e: &Value) -> &str {
    e.get("entity_id").and_then(Value::as_str).unwrap_or_default()
}

/// `name` may be a plain string or a `LanguageText` map (`{"en": "..."}`); fall back to the id.
fn entity_name(e: &Value) -> String {
    match e.get("name") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Object(m)) => m
            .get("en")
            .or_else(|| m.values().next())
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        _ => e.get("entity_id").and_then(Value::as_str).unwrap_or("?").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(volume_db(62), "−18 dB");
        assert_eq!(volume_db(80), "0 dB");
        assert_eq!(time_left(5512.0), "1:32 left");
        assert_eq!(time_left(600.0), "10 min left");
        assert_eq!(format_badge("DOLBY DIGITAL"), ("dolby", "Digital".to_string()));
        assert_eq!(format_badge("DTS HD MA 7.1"), ("dts", "HD MA 7.1".to_string()));
        assert_eq!(format_badge("STEREO"), ("", "Stereo".to_string()));
    }
}
