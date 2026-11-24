//! remote-ui: a single-purpose theater remote. The receiver's volume and input come from one
//! entity, Kodi's playback from another; the screen shows whichever the receiver is switched to.
//! The tokio runtime runs on its own thread and pushes a `Model` into the Slint event loop via
//! `invoke_from_event_loop`; UI callbacks flow back over a channel. See docs/plan.md.

mod audio;
mod backlight;
mod device;
#[cfg(feature = "device")]
mod evdev_input;
mod feedback;
#[cfg(feature = "device")]
mod frames;
mod input;
mod keyboard;
mod kodi;
mod lava;
mod net;
mod settings;
mod shots;
mod slider;

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
const SLIDER_HOLD: Duration = Duration::from_millis(1500);
/// The longest the slider's level is shown while the receiver has not yet reported it.
const SLIDER_SETTLE: Duration = Duration::from_secs(6);
/// How long an input picked here is shown before the receiver's own report wins again.
const SWITCH_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a power change made here is believed before the receiver's own report wins again.
const POWER_TIMEOUT: Duration = Duration::from_secs(120);
/// On the charger, the screen and keys go dark after this long without a touch or key.
const CHARGER_REST: Duration = Duration::from_secs(10);

pub enum UiAction {
    Command(String),
    Key(String),
    SelectSource(String),
    Power(String),
    PowerHeld,
    WifiScan,
    WifiBand,
    WifiJoin { ssid: String, hex: String, password: String },
    Backlight(backlight::Keys),
    Seek(i32),
    /// 0 audio, 1 subtitles
    OpenPicker(i32),
    PickTrack(i32, i32),
    Slider(slider::Gesture),
    Display { brightness: i32, auto: bool },
}

/// Results of work that finishes later than the request that started it.
enum Later {
    Wifi(String, Vec<net::Network>),
    Integrations(HashMap<String, bool>),
    /// The core's current list of configured entities.
    Entities(Vec<Value>),
    JoinDone(bool),
    /// Kodi's subtitle streams, as (stream index or None for off, label), and which is on.
    Subtitles(Vec<(Option<i64>, String)>, usize),
    /// Demo only: the receiver has caught up with an input change.
    Switched,
    /// The slider's throttled volume command is due.
    SendVolume,
}

/// Everything the screen shows, independent of where it came from (core or demo).
#[derive(Clone, Debug, Default)]
pub struct Model {
    pub volume: i32,
    pub muted: bool,
    pub source: String,
    pub sources: Vec<String>,
    pub sound_mode: String,
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
            sound_mode: "DOLBY DIGITAL".into(),
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
    fn on_kodi(&self) -> bool {
        self.source.is_empty() || self.source.eq_ignore_ascii_case("kodi")
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_ansi(cfg!(not(feature = "device")))
        .init();

    for (key, value) in &settings::get().env {
        std::env::set_var(key, value);
    }
    #[cfg(feature = "device")]
    log_display_facts();
    #[cfg(feature = "device")]
    select_device_renderer();
    let app = AppWindow::new()?;
    app.global::<Theme>().set_touch_only(cfg!(feature = "device"));
    app.set_grain(grain());
    #[cfg(feature = "device")]
    frames::install(&app);
    // UC_ACCENT pins one colour; without it the colour follows the input (see `apply`).
    if let Some(accent) = std::env::var("UC_ACCENT").ok().as_deref().and_then(parse_color) {
        set_accent(&app, accent);
    }

    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1));
    let frame = arg("--frame").map(|p| device::Device::load(p.as_ref())).transpose()?;
    let (act_tx, mut act_rx) = mpsc::unbounded_channel();
    #[cfg(feature = "device")]
    evdev_input::start(&app, act_tx.clone());
    wire_keyboard(&app, act_tx.clone());
    app.on_capture({
        let weak = app.as_weak();
        move |which| {
            if let Some(app) = weak.upgrade() {
                capture(&app, which);
            }
        }
    });
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
    app.on_haptic(|name| feedback::ui_event(&name));
    let lab = Rc::new(RefCell::new(feedback::Lab::default()));
    let show_lab = {
        let weak = app.as_weak();
        let lab = lab.clone();
        move || {
            let Some(app) = weak.upgrade() else { return };
            let lab = lab.borrow();
            let rows: Vec<Choice> = lab
                .rows()
                .into_iter()
                .map(|r| Choice { label: r.label.into(), detail: r.detail.into(), value: r.value.into(), chevron: r.opens, ..Default::default() })
                .collect();
            app.set_lab_title(lab.title().into());
            app.set_lab_rows(ModelRc::new(VecModel::from(rows)));
        }
    };
    show_lab();
    app.on_lab_pick({
        let (lab, show_lab) = (lab.clone(), show_lab.clone());
        move |i| {
            let moved = lab.borrow_mut().pick(i as usize);
            show_lab();
            moved
        }
    });
    app.on_lab_back(move || {
        let focus = lab.borrow_mut().back();
        show_lab();
        focus.map_or(-1, |f| f as i32)
    });
    app.on_wifi_scan({
        let tx = act_tx.clone();
        move || {
            let _ = tx.send(UiAction::WifiScan);
        }
    });
    app.on_wifi_band({
        let tx = act_tx.clone();
        move || {
            let _ = tx.send(UiAction::WifiBand);
        }
    });
    app.on_wifi_join({
        let tx = act_tx.clone();
        move |ssid, hex, password| {
            let _ = tx.send(UiAction::WifiJoin { ssid: ssid.into(), hex: hex.into(), password: password.into() });
        }
    });
    app.on_seek({
        let tx = act_tx.clone();
        move |by| {
            let _ = tx.send(UiAction::Seek(by));
        }
    });
    app.on_open_picker({
        let tx = act_tx.clone();
        move |kind| {
            let _ = tx.send(UiAction::OpenPicker(kind));
        }
    });
    app.on_pick_track({
        let tx = act_tx.clone();
        move |kind, i| {
            let _ = tx.send(UiAction::PickTrack(kind, i));
        }
    });
    app.on_activity(poke);
    app.on_display_changed({
        let (tx, weak) = (act_tx.clone(), app.as_weak());
        move || {
            if let Some(app) = weak.upgrade() {
                let _ = tx.send(UiAction::Display { brightness: app.get_brightness(), auto: app.get_auto_brightness() });
            }
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
        let mut last = None;
        let timer = slint::Timer::default();
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(150), move || {
            let Some(app) = weak.upgrade() else { return };
            let (state, accent) = backlight::observe(&app);
            let keys = backlight::keys_for(state, accent);
            if last != Some(keys) {
                last = Some(keys);
                let _ = tx.send(UiAction::Backlight(keys));
            }
        });
        timer
    };
    let _rest = rest_on_charger(&app);
    let _lava = animate_lava(&app);
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

/// The stage as it looks right now, alone and unreceded.
fn snapshot_stage(app: &AppWindow) -> Option<image::RgbaImage> {
    app.set_capturing(1);
    let shot = app.window().take_snapshot();
    app.set_capturing(0);
    let shot = shot.map_err(|e| tracing::warn!("stage snapshot: {e}")).ok()?;
    image::RgbaImage::from_raw(shot.width(), shot.height(), shot.as_bytes().to_vec())
}

fn soften(img: &image::RgbaImage, sigma: f32) -> slint::Image {
    let small = image::imageops::resize(img, 60, 100, image::imageops::FilterType::Triangle);
    slint::Image::from_rgba8(pixels(image::imageops::blur(&small, sigma)))
}

/// What decides which main stage is showing.
#[derive(Clone, PartialEq)]
struct StageKey {
    receiver_on: bool,
    playing: bool,
    source: String,
}

thread_local!(static STAGE: RefCell<Option<StageKey>> = const { RefCell::new(None) });

/// Apply `change`, and if it moved the window to a different main stage while the stage is in
/// view, blur through from the old one to the new.
fn stage_change(app: &AppWindow, m: &Model, change: impl FnOnce()) {
    let key = StageKey { receiver_on: m.receiver_on, playing: m.on_kodi() && !m.title.is_empty(), source: m.source.clone() };
    let before = STAGE.with(|s| s.replace(Some(key.clone())));
    let visible = app.get_page() == 0 && app.get_lit() && app.get_capturing() == 0;
    if before.is_none() || before.as_ref() == Some(&key) || !visible {
        change();
        return;
    }
    let old = snapshot_stage(app);
    change();
    let new = snapshot_stage(app);
    let (Some(old), Some(new)) = (old, new) else { return };
    let plain = before.is_some_and(|b| b.receiver_on == key.receiver_on && b.source == key.source);
    app.set_swap_plain(plain);
    app.set_swap_out_blur(soften(&old, 2.2));
    app.set_swap_in_blur(soften(&new, 2.2));
    app.set_swap_out(slint::Image::from_rgba8(pixels(old)));
    app.invoke_hold_swap();
    let weak = app.as_weak();
    slint::Timer::single_shot(Duration::ZERO, move || {
        if let Some(app) = weak.upgrade() {
            app.invoke_run_swap();
        }
    });
}

/// A soft, blurred picture of the screen for a layer that is about to recede: 0 the stage alone,
/// 1 everything under the overlays.
pub fn capture(app: &AppWindow, which: i32) {
    app.set_capturing(which + 1);
    let shot = app.window().take_snapshot();
    app.set_capturing(0);
    let shot = match shot {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("snapshot for blur: {e}");
            return;
        }
    };
    let Some(full) = image::RgbaImage::from_raw(shot.width(), shot.height(), shot.as_bytes().to_vec()) else { return };
    let small = image::imageops::resize(&full, 60, 100, image::imageops::FilterType::Triangle);
    let soft = image::imageops::blur(&small, 1.6);
    let img = slint::Image::from_rgba8(pixels(soft));
    if which == 0 {
        app.set_stage_blur(img);
    } else {
        app.set_world_blur(img);
    }
}

static LAST_INPUT: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);

/// A key or touch: the charger's countdown to dark starts over.
pub fn poke() {
    *LAST_INPUT.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
}

fn since_input() -> Duration {
    LAST_INPUT.lock().unwrap_or_else(|e| e.into_inner()).map_or(Duration::MAX, |t| t.elapsed())
}

/// On the charger nobody is holding the remote, so the screen and keys go dark soon. The core's
/// own display timeout still applies on top of this.
fn rest_on_charger(app: &AppWindow) -> slint::Timer {
    let weak = app.as_weak();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(500), move || {
        let Some(app) = weak.upgrade() else { return };
        // Typing a Wi-Fi password is slow; never black out the keyboard.
        let busy = app.get_page() == 6 || app.get_power_overlay();
        if app.get_charging() && app.get_lit() && !busy && since_input() >= CHARGER_REST {
            app.set_resting(true);
            app.set_charge_card(false);
        }
    });
    timer
}

/// Redraws the menu's background while the menu is on screen, at a calm 20 fps.
pub fn animate_lava(app: &AppWindow) -> slint::Timer {
    let weak = app.as_weak();
    let start = Instant::now();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(50), move || {
        let Some(app) = weak.upgrade() else { return };
        if !app.get_lava_on() {
            return;
        }
        let a = app.global::<Theme>().get_accent();
        let colours = lava::palette((a.red(), a.green(), a.blue()), settings::get().lava.unwrap_or(1.0));
        app.set_lava(slint::Image::from_rgb8(lava::render(start.elapsed().as_secs_f32() * 0.6, colours)));
    });
    timer
}

/// What the system says about the panel, once, for diagnosing display behaviour from the logs.
#[cfg(feature = "device")]
fn log_display_facts() {
    let read = |p: &std::path::Path| std::fs::read_to_string(p).map(|s| s.trim().replace('\n', " | ")).ok();
    if let Some(v) = read("/proc/version".as_ref()) {
        tracing::info!("kernel: {v}");
    }
    for dir in ["/sys/class/drm", "/sys/class/graphics", "/sys/class/backlight"] {
        let Ok(entries) = std::fs::read_dir(dir) else {
            tracing::info!("{dir}: not readable");
            continue;
        };
        for entry in entries.flatten() {
            let facts: Vec<String> = ["status", "enabled", "dpms", "modes", "mode", "virtual_size", "name", "brightness", "max_brightness"]
                .iter()
                .filter_map(|f| Some(format!("{f}={}", read(&entry.path().join(f)).filter(|v| !v.is_empty())?)))
                .collect();
            tracing::info!("{}: {}", entry.path().display(), facts.join("; "));
        }
    }
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
    // Artwork for the demo's film, from local files, so previews show the real layout.
    let art = |var: &str| image::open(std::env::var_os(var)?).map_err(|e| tracing::warn!("{var}: {e}")).ok();
    let poster = art("UC_DEMO_POSTER");
    if let Some(p) = &poster {
        app.set_poster(slint::Image::from_rgba8(poster_pixels(p)));
    }
    if let Some(b) = art("UC_DEMO_FANART").or(poster) {
        app.set_backdrop(slint::Image::from_rgba8(backdrop_pixels(&b)));
    }
    app.set_haptics("On".into());
    app.set_screen_off_after("60 s".into());
    app.set_sleep_after("3 min".into());
}

/// The Kodi driver swaps the title for the audio/subtitle track names or the chapter name for a
/// few seconds whenever one changes (its "show track name" setting). This holds the real title
/// steady: a different title for the same film has to last before it is believed.
#[derive(Default)]
struct StableTitle {
    media: String,
    shown: String,
    pending: Option<(String, Instant)>,
}

impl StableTitle {
    const SETTLE: Duration = Duration::from_secs(7);

    fn feed(&mut self, media: &str, title: &str, tracks: &str, now: Instant) {
        // The track-name flash is the same text the driver reports as the artist.
        let is_tracks = !tracks.is_empty() && title == tracks;
        if media != self.media {
            self.media = media.to_string();
            self.shown = if is_tracks { String::new() } else { title.to_string() };
            self.pending = None;
        } else if is_tracks || title == self.shown {
            self.pending = None;
        } else if self.shown.is_empty() {
            self.shown = title.to_string();
        } else {
            match &self.pending {
                Some((t, since)) if t == title && now.duration_since(*since) >= Self::SETTLE => {
                    self.shown = title.to_string();
                    self.pending = None;
                }
                Some((t, _)) if t == title => {}
                _ => self.pending = Some((title.to_string(), now)),
            }
        }
    }
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
    wifi_band: String,
    scanning: bool,
    integrations: HashMap<String, bool>,
    title: StableTitle,
    /// The network id of a join in progress, and when it started.
    joining: Option<(Option<i64>, Instant)>,
    /// What each row of the subtitle picker selects.
    subtitle_ids: Vec<Option<i64>>,
    /// The level the slider last asked for. The receiver takes a moment to report it back, and
    /// steps taken meanwhile must build on what was asked, not on the stale report.
    slider_volume: Option<(i32, Instant)>,
    charging: bool,
    /// An input picked here, shown straight away until the receiver reports it (or gives up).
    pending_source: Option<(String, Instant)>,
    /// The same for power: the receiver can take a minute to report that it has come on.
    pending_power: Option<(bool, Instant)>,
    /// And for mute, so a second double tap undoes the first even before the receiver reports.
    pending_mute: Option<(bool, Instant)>,
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

    /// Fetch the artwork whenever Kodi reports a different image URL: the poster, and behind it
    /// the item's backdrop (the poster again when it has none), blurred.
    fn sync_poster(&mut self, weak: &slint::Weak<AppWindow>) {
        let url = attr_str(self.target(Target::Screen), "media_image_url");
        if url == self.poster_url {
            return;
        }
        self.poster_url = url.clone();
        let weak = weak.clone();
        tokio::task::spawn_blocking(move || {
            let poster = (!url.is_empty()).then(|| fetch_image(&url).map_err(|e| tracing::warn!("poster {url}: {e:#}")).ok()).flatten();
            let fanart = kodi::base(&url)
                .and_then(|base| kodi::playing(&base).map_err(|e| tracing::info!("kodi json-rpc: {e:#}")).ok())
                .map(|p| p.fanart)
                .filter(|f| !f.is_empty())
                .and_then(|f| fetch_image(&f).map_err(|e| tracing::warn!("fanart {f}: {e:#}")).ok());
            let backdrop = fanart.as_ref().or(poster.as_ref()).map(backdrop_pixels);
            let poster = poster.as_ref().map(poster_pixels);
            ui(&weak, move |app| {
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
            .find(|e| !suffix.is_empty() && entity_id(e).ends_with(suffix.as_str()))
            .and_then(|e| e.pointer("/attributes/value"))
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
        let ending = |suffix: &str| self.entities.iter().find(|e| !suffix.is_empty() && entity_id(e).ends_with(suffix));
        let colour = ending(&cfg.keys_light).filter(|l| attr_str(Some(l), "state") == "ON").map(|light| {
            let (h, s, v) = (
                attr_f64(Some(light), "hue"),
                attr_f64(Some(light), "saturation") / 255.0,
                attr_f64(Some(light), "brightness") / 255.0,
            );
            // HSV to RGB, with the light's brightness as the value.
            let f = |n: f64| {
                let k = (n + h / 60.0) % 6.0;
                ((v - v * s * k.min(4.0 - k).clamp(0.0, 1.0)) * 255.0).round() as u8
            };
            Rgb(f(5.0), f(3.0), f(1.0))
        });
        let mode = match attr_str(ending(&cfg.keys_mode), "current_option").to_lowercase().as_str() {
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
        let media = format!("{}|{}", attr_f64(screen, "media_duration"), attr_str(screen, "media_image_url"));
        let (title, tracks) = (attr_str(screen, "media_title"), attr_str(screen, "media_artist"));
        self.title.feed(&media, &title, &tracks, Instant::now());
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
        let reported = attr_str(receiver, "source");
        let pending = self.pending_source.as_ref().filter(|(name, at)| *name != reported && at.elapsed() < SWITCH_TIMEOUT);
        let source = pending.map_or(reported, |(name, _)| name.clone());
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
            keys: self.keys(),
            wifi_ssid: self.wifi_ssid.clone(),
            wifi: self.wifi.clone(),
            wifi_band: self.wifi_band.clone(),
            scanning: self.scanning,
            status,
            warning,
            demo_ha_down: false,
            volume: {
                let reported = attr_f64(volume_from, "volume").round() as i32;
                match self.slider_volume {
                    // Until the receiver has caught up, its reports are old levels on the way.
                    Some((v, at)) if at.elapsed() < SLIDER_HOLD || (v != reported && at.elapsed() < SLIDER_SETTLE) => v,
                    _ => reported,
                }
            },
            muted: {
                let reported = volume_from.and_then(|e| e.pointer("/attributes/muted")).and_then(Value::as_bool).unwrap_or(false);
                match self.pending_mute {
                    Some((want, at)) if want != reported && at.elapsed() < SLIDER_SETTLE => want,
                    _ => reported,
                }
            },
            source,
            sources: receiver
                .and_then(|e| e.pointer("/attributes/source_list"))
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).filter(|s| !settings::get().hides(s)).map(String::from).collect())
                .unwrap_or_default(),
            sound_mode: attr_str(receiver, "sound_mode"),
            switching: pending.is_some(),
            // The Kodi driver keeps the last title, artwork and position after a stop, so only the
            // playback state says whether anything is on.
            title: if matches!(state.as_str(), "PLAYING" | "PAUSED" | "BUFFERING") { self.title.shown.clone() } else { String::new() },
            paused,
            duration: attr_f64(screen, "media_duration"),
            position,
            media_format: audio_format(&attr_str(screen, "sound_mode")),
            media_sub: episode(&attr_str(screen, "media_type"), &attr_str(screen, "media_artist")),
            audio_tracks: screen
                .and_then(|e| e.pointer("/attributes/sound_mode_list"))
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
                .unwrap_or_default(),
            audio_track: attr_str(screen, "sound_mode"),
            battery: self.battery,
            charging: self.charging,
            awake: self.awake,
            receiver_on: {
                let reported = receiver.is_none_or(|r| attr_str(Some(r), "state") != "OFF");
                match self.pending_power {
                    Some((want, at)) if want != reported && at.elapsed() < POWER_TIMEOUT => want,
                    _ => reported,
                }
            },
            volume_max: self.volume_max(),
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
    let charger = core.request("get_battery_charger", json!({})).await.map(|r| r.data).unwrap_or(Value::Null);
    tracing::info!(%power, %charger, "power");
    let battery = power.pointer("/battery/capacity").or(charger.get("capacity")).and_then(Value::as_i64).unwrap_or(100) as i32;
    let charging = on_power(&power).or_else(|| on_power(&charger)).unwrap_or(false);
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
        wifi_band: String::new(),
        scanning: false,
        integrations: HashMap::new(),
        joining: None,
        subtitle_ids: Vec::new(),
        slider_volume: None,
        charging,
        pending_source: None,
        pending_power: None,
        pending_mute: None,
        title: StableTitle::default(),
        awake: is_awake(power.get("mode").and_then(Value::as_str)),
        screen,
        entities,
    };
    load_settings(weak, &core).await;
    (session.wifi_ssid, session.wifi) = net::wifi(&core).await;
    session.wifi_band = wifi_band(&core).await;
    session.integrations = net::integrations(&core).await;
    session.settle_title();
    session.sync_poster(weak);
    session.position = attr_f64(session.target(Target::Screen), "media_position");
    tracing::info!(screen = ?session.screen, volume = ?session.volume, "targets");

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
    let pace = |send: slider::Send, session: &Session, moves: u32| match send {
        slider::Send::Now(v) => {
            tracing::info!("slider volume {v} (from {moves} moves)");
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
                            if session.entity(entity_id(&e)).is_none() {
                                tracing::info!("new entity {}", entity_id(&e));
                                session.entities.push(e);
                            }
                        }
                    }
                    Later::Subtitles(rows, current) => {
                        session.subtitle_ids = rows.iter().map(|r| r.0).collect();
                        let labels: Vec<String> = rows.into_iter().map(|r| r.1).collect();
                        show_tracks(weak, labels.iter().map(|l| (l.clone(), String::new())).collect(), current);
                    }
                    Later::Switched => {}
                    Later::SendVolume => {
                        let send = volume_gate.due(Instant::now());
                        let moves = if matches!(send, slider::Send::Now(_)) { std::mem::take(&mut volume_gate.merged) } else { 0 };
                        pace(send, &session, moves);
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
                if ev.msg == "power_mode_change" {
                    session.awake = is_awake(ev.data.get("mode").and_then(Value::as_str));
                    if let Some(on) = on_power(&ev.data) {
                        session.charging = on;
                    }
                    render(weak, &session.model());
                }
                if ev.msg == "battery_status" {
                    if let Some(c) = ev.data.get("capacity").and_then(Value::as_i64) {
                        session.battery = c as i32;
                    }
                    if let Some(on) = on_power(&ev.data) {
                        session.charging = on;
                    }
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
                if ev.msg == "entity_change" && session.volume.as_deref() == ev.data.get("entity_id").and_then(Value::as_str) {
                    if let Some(v) = ev.data.pointer("/new_state/attributes/volume").and_then(Value::as_f64) {
                        let send = volume_gate.reported(v.round() as i32, Instant::now());
                        let moves = if matches!(send, slider::Send::Now(_)) { std::mem::take(&mut volume_gate.merged) } else { 0 };
                        pace(send, &session, moves);
                    }
                }
                if ev.msg == "entity_change" && session.apply_change(&ev.data) {
                    session.settle_title();
                    session.sync_poster(weak);
                    render(weak, &session.model());
                }
            }
            Some(action) = actions.recv() => {
                let (target, cmd, params) = match action {
                    UiAction::Command(cmd) => (Target::Screen, cmd, Value::Null),
                    UiAction::SelectSource(name) => {
                        session.pending_source = Some((name.clone(), Instant::now()));
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
                        session.pending_power = Some((!on, Instant::now()));
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
                                Ok(r) if (200..300).contains(&r.code) => {}
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
                    UiAction::OpenPicker(0) => {
                        let m = session.model();
                        let current = m.audio_tracks.iter().position(|t| *t == m.audio_track).unwrap_or(usize::MAX);
                        show_tracks(weak, m.audio_tracks.iter().map(|t| split_track(t)).collect(), current);
                        continue;
                    }
                    UiAction::OpenPicker(_) => {
                        show_tracks(weak, Vec::new(), usize::MAX);
                        let Some(base) = kodi::base(&session.poster_url) else { continue };
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
                    UiAction::PickTrack(0, i) => {
                        let Some(mode) = session.model().audio_tracks.get(i as usize).cloned() else { continue };
                        (Target::Screen, "select_sound_mode".to_string(), json!({ "mode": mode }))
                    }
                    UiAction::PickTrack(_, i) => {
                        let (Some(base), Some(id)) = (kodi::base(&session.poster_url), session.subtitle_ids.get(i as usize).copied()) else { continue };
                        tokio::task::spawn_blocking(move || {
                            if let Err(e) = kodi::set_subtitle(&base, id) {
                                tracing::warn!("set subtitle: {e:#}");
                            }
                        });
                        continue;
                    }
                    UiAction::Slider(slider::Gesture::Steps(n)) => {
                        let v = slide(session.volume_base(), n, session.volume_max());
                        if v == session.volume_base() {
                            feedback::play(feedback::SLIDER_LIMIT);
                        }
                        session.slider_volume = Some((v, Instant::now()));
                        flash_volume(weak, &overlay);
                        render(weak, &session.model());
                        let send = volume_gate.offer(v, Instant::now());
                        let moves = if matches!(send, slider::Send::Now(_)) { std::mem::take(&mut volume_gate.merged) } else { 0 };
                        pace(send, &session, moves);
                        continue;
                    }
                    UiAction::Slider(slider::Gesture::DoubleTap) => {
                        feedback::play(feedback::SLIDER_MUTE);
                        flash_volume(weak, &overlay);
                        (Target::Volume, "mute_toggle".to_string(), Value::Null)
                    }
                    UiAction::WifiBand => {
                        let Some(next) = (match session.wifi_band.as_str() { "a" => Some("b"), "b" => Some("a"), _ => None }) else { continue };
                        let cfg = json!({ "wifi": { "band": next } });
                        match core.request("set_network_cfg", cfg).await {
                            Ok(r) if (200..300).contains(&r.code) => {
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
                // The receiver reports late, so the toggle follows what this remote last asked for.
                let cmd = if cmd == "mute_toggle" && target == Target::Volume {
                    let (muted, toggles) = (session.model().muted, has_feature(entity, "mute_toggle"));
                    session.pending_mute = Some((!muted, Instant::now()));
                    render(weak, &session.model());
                    if toggles { cmd } else { if muted { "unmute" } else { "mute" }.to_string() }
                } else {
                    cmd
                };
                let Some(entity) = session.target(target) else { continue };
                commands.send(entity, cmd, params);
            }
        }
    }
    Ok(())
}

fn band_label(band: &str) -> Option<&'static str> {
    match band {
        "a" => Some("5 GHz"),
        "b" => Some("2.4 GHz"),
        _ => None,
    }
}

/// The Denon tops out at 98 (+18 dB).
const MAX_VOLUME: i32 = 98;

/// Whether the remote is on its charger, from any message that says.
fn on_power(v: &Value) -> Option<bool> {
    v.get("power_supply").or_else(|| v.pointer("/battery/power_supply")).and_then(Value::as_bool)
}

/// "French VF - AAC-LC stereo" → ("French VF", "AAC-LC stereo")
fn split_track(t: &str) -> (String, String) {
    match t.split_once(" - ") {
        Some((name, codec)) => (name.trim().to_string(), codec.split('(').next().unwrap_or_default().trim().to_string()),
        None => (t.to_string(), String::new()),
    }
}

/// "S1E4" for an episode → "Season 1 · Episode 4"
fn episode(media_type: &str, artist: &str) -> String {
    if !media_type.eq_ignore_ascii_case("episode") {
        return String::new();
    }
    let up = artist.to_uppercase();
    let Some((s, e)) = up.strip_prefix('S').and_then(|r| r.split_once('E')) else { return String::new() };
    match (s.parse::<u32>(), e.parse::<u32>()) {
        (Ok(s), Ok(e)) => format!("Season {s} · Episode {e}"),
        _ => String::new(),
    }
}

pub fn demo_tracks(m: &Model, kind: i32, subtitle: usize) -> (Vec<(String, String)>, usize) {
    if kind == 0 {
        let current = m.audio_tracks.iter().position(|t| *t == m.audio_track).unwrap_or(usize::MAX);
        (m.audio_tracks.iter().map(|t| split_track(t)).collect(), current)
    } else {
        let rows = ["Off", "English", "English SDH", "French · Forced"].map(|l| (l.to_string(), String::new()));
        (rows.to_vec(), subtitle)
    }
}

/// Fill the picker. `current` beyond the list marks none.
pub fn show_tracks(weak: &slint::Weak<AppWindow>, rows: Vec<(String, String)>, current: usize) {
    ui(weak, move |app| {
        let rows: Vec<Choice> = rows
            .into_iter()
            .enumerate()
            .map(|(i, (label, detail))| Choice { label: label.into(), detail: detail.into(), checked: i == current, ..Default::default() })
            .collect();
        let focus = rows.iter().position(|r| r.checked).unwrap_or(0) as i32;
        app.set_tracks(ModelRc::new(VecModel::from(rows)));
        if app.get_page() == 9 {
            app.set_focus_index(focus);
        }
    });
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

/// The preferred Wi-Fi band ("a" or "b"), or empty when the core does not say.
async fn wifi_band(core: &coreapi::Core) -> String {
    core.request("get_network_cfg", json!({}))
        .await
        .ok()
        .and_then(|r| r.data.pointer("/wifi/band").and_then(Value::as_str).map(String::from))
        .unwrap_or_default()
}

/// Plain HTTP only: Kodi serves its artwork from its own web server on the LAN.
fn fetch_image(url: &str) -> anyhow::Result<image::DynamicImage> {
    use std::io::Read;
    let mut bytes = Vec::new();
    ureq::get(url).timeout(Duration::from_secs(8)).call()?.into_reader().take(24 << 20).read_to_end(&mut bytes)?;
    Ok(image::load_from_memory(&bytes)?)
}

type Pixels = slint::SharedPixelBuffer<slint::Rgba8Pixel>;

fn pixels(img: image::RgbaImage) -> Pixels {
    Pixels::clone_from_slice(img.as_raw(), img.width(), img.height())
}

pub fn poster_pixels(img: &image::DynamicImage) -> Pixels {
    pixels(img.resize_to_fill(384, 576, image::imageops::FilterType::Triangle).to_rgba8())
}

/// Monochrome film grain to lay over the backdrop: white and black specks, mostly transparent.
/// Drawn once; a fixed seed keeps it the same from run to run.
pub fn grain() -> slint::Image {
    let (w, h) = (480u32, 800u32);
    let mut buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(w, h);
    let mut x: u32 = 0x9E37_79B9;
    for p in buf.make_mut_slice() {
        // xorshift32
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        let v = (x & 0xFF) as u8;
        let a = ((x >> 8) & 0x1F) as u8;
        *p = slint::Rgba8Pixel { r: v, g: v, b: v, a };
    }
    slint::Image::from_rgba8(buf)
}

/// Small, blurred, darker and a little greyer, so it sits behind the controls. The window scales
/// it up smoothly, which softens it further.
pub fn backdrop_pixels(img: &image::DynamicImage) -> Pixels {
    let small = img.resize_to_fill(120, 200, image::imageops::FilterType::Triangle).to_rgba8();
    let mut soft = image::imageops::blur(&small, 3.2);
    for p in soft.pixels_mut() {
        let [r, g, b, _] = p.0;
        let grey = 0.3 * r as f32 + 0.59 * g as f32 + 0.11 * b as f32;
        let mix = |c: u8| ((c as f32 * 0.5 + grey * 0.5) * 0.62) as u8;
        p.0 = [mix(r), mix(g), mix(b), 255];
    }
    pixels(soft)
}

/// The menu's current values. Each is optional: a core that lacks a message just leaves a blank.
async fn load_settings(weak: &slint::Weak<AppWindow>, core: &coreapi::Core) {
    let get = |msg: &'static str| async move {
        core.request(msg, json!({})).await.ok().filter(|r| (200..300).contains(&r.code)).map(|r| r.data)
    };
    let (display, haptic, power, sound) =
        tokio::join!(get("get_display_cfg"), get("get_haptic_cfg"), get("get_power_saving_cfg"), get("get_sound_cfg"));
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
                Later::Integrations(_) | Later::Entities(_) | Later::Subtitles(..) => {}
                Later::Switched => m.switching = false,
                Later::SendVolume => {}
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
                UiAction::WifiBand => m.wifi_band = if m.wifi_band == "a" { "b".into() } else { "a".into() },
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
                UiAction::Backlight(..) | UiAction::Display { .. } => continue,
                UiAction::Seek(by) => m.position = (m.position + by as f64).clamp(0.0, m.duration),
                UiAction::OpenPicker(kind) => {
                    let (rows, current) = demo_tracks(&m, kind, demo_subtitle);
                    show_tracks(weak, rows, current);
                }
                UiAction::PickTrack(0, i) => {
                    if let Some(t) = m.audio_tracks.get(i as usize) {
                        m.audio_track = t.clone();
                        m.media_format = audio_format(t);
                    }
                }
                UiAction::PickTrack(_, i) => demo_subtitle = i as usize,
                UiAction::Slider(slider::Gesture::Steps(n)) => {
                    let before = m.volume;
                    m.volume = slide(m.volume, n, m.volume_max);
                    if m.volume == before {
                        feedback::play(feedback::SLIDER_LIMIT);
                    }
                    m.muted = false;
                    flash_volume(weak, &overlay);
                }
                UiAction::Slider(slider::Gesture::DoubleTap) => {
                    feedback::play(feedback::SLIDER_MUTE);
                    m.muted = !m.muted;
                    flash_volume(weak, &overlay);
                }
                UiAction::Key(text) if text == "c" => m.charging = !m.charging,
                UiAction::Key(text) if text == "x" => m.demo_ha_down = !m.demo_ha_down,
                UiAction::SelectSource(name) => {
                    m.source = name;
                    m.switching = true;
                    after(1800, Later::Switched);
                }
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
    ui(weak, |app| {
        if !app.get_volume_overlay() {
            capture(&app, 1);
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

type Queued = (String, Value, Instant);

/// The longest a command waits for the previous one's answer before going out anyway.
const PRESS_GAP: Duration = Duration::from_millis(40);

/// One queue per entity, for one core session: commands for an entity go out in the order they
/// were made, each after the previous one is answered or `PRESS_GAP` has passed.
struct Commands {
    core: coreapi::Core,
    queues: RefCell<HashMap<String, mpsc::UnboundedSender<Queued>>>,
}

impl Commands {
    fn new(core: coreapi::Core) -> Self {
        Self { core, queues: RefCell::new(HashMap::new()) }
    }

    fn send(&self, entity: &Value, cmd: String, params: Value) {
        let id = entity_id(entity).to_string();
        let mut queues = self.queues.borrow_mut();
        let tx = queues.entry(id.clone()).or_insert_with(|| {
            let (tx, mut rx) = mpsc::unbounded_channel::<Queued>();
            let (core, name) = (self.core.clone(), entity_name(entity));
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
                            Ok(r) if (200..300).contains(&r.code) => "ok".to_string(),
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

/// A model change that alters what is on screen starts a transition; the playback position
/// ticking along does not count.
#[cfg(feature = "device")]
fn kick_if_changed(m: &Model) {
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
pub fn apply(app: &AppWindow, m: &Model) {
    stage_change(app, m, || apply_now(app, m));
}

fn apply_now(app: &AppWindow, m: &Model) {
    #[cfg(feature = "device")]
    kick_if_changed(m);
    let now = chrono::Local::now();
    let logos = app.global::<Logos>();

    app.set_clock(now.format("%H:%M").to_string().into());
    if std::env::var_os("UC_ACCENT").is_none() {
        if let Some(accent) = settings::get().accent_for(&m.source).and_then(parse_color) {
            set_accent(app, accent);
        }
    }
    app.set_awake(m.awake);
    backlight::set_ha(m.keys);
    app.set_battery(m.battery);
    if m.charging != app.get_charging() {
        // Put on the charger: show the battery, and start counting down to dark from now.
        if m.charging {
            capture(app, 1);
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
    app.set_time_left(if known { format!("−{}", clock(left)).into() } else { SharedString::new() });
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

/// The strip is 100 steps from silence to the receiver's limit; never less than one level a step.
fn slide(from: i32, steps: i32, max: i32) -> i32 {
    let by = (steps as f64 * max as f64 / 100.0).round() as i32;
    let by = if by == 0 { steps.signum() } else { by };
    (from + by).clamp(0, max)
}

fn volume_percent(volume: i32, max: i32) -> i32 {
    ((volume.max(0) as f64 / max.max(1) as f64) * 100.0).round().min(100.0) as i32
}

/// Denon reports volume as dB + 80, so the receiver's own dB reading is the level minus 80.
fn volume_db(volume: i32) -> String {
    match volume - 80 {
        db if db < 0 => format!("−{} dB", -db),
        0 => "0 dB".to_string(),
        db => format!("+{db} dB"),
    }
}

/// 4328 → "1:12:08", 548 → "9:08"
fn clock(secs: f64) -> String {
    let s = secs.max(0.0).round() as i64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Kodi reports its audio stream as "English DTS-HD MA 5.1 - DTS-HD MA 5.1(side)": language and
/// stream name, then the codec with its channel layout. The codec part is what the badge shows.
fn audio_format(sound_mode: &str) -> String {
    let codec = sound_mode.rsplit(" - ").next().unwrap_or_default();
    codec.split('(').next().unwrap_or_default().trim().to_string()
}

/// The badge for Kodi's audio stream: a logo id and what the logo does not already say. The
/// stream name is free text from the file (often "DTS-HD MA 5.1", sometimes the driver's fallback
/// such as "Truehd_Atmos 8 channels"), so this matches families by keyword, most specific first.
fn codec_badge(stream: &str) -> (&'static str, String) {
    let s = format!(" {} ", stream.to_lowercase().replace(['_', '(', ')', ','], " "));
    let has = |words: &[&str]| words.iter().any(|w| s.contains(w));
    let ch = channels(&s);
    let with = |prefix: &str| [prefix, ch.as_str()].iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(" ");
    // Dolby and DTS show only their marks; the label names the format.
    if has(&["atmos"]) {
        let carrier = if has(&["truehd", "true hd"]) { "TrueHD Atmos" } else if has(&["dd+", "ddp", "eac3", "e-ac-3", "digital plus"]) { "Digital Plus Atmos" } else { "Atmos" };
        return ("dolby", with(carrier));
    }
    if has(&["dts:x", "dtsx", "dts-x", " ma x ", "dts x "]) {
        return ("dts", with(if has(&["imax"]) { "X IMAX" } else { "X" }));
    }
    if has(&["dts-hd ma", "dtshd ma", "dts-hd master", "dts hd ma", "dts ma ", "dts-ma"]) {
        return ("dts", with("HD MA"));
    }
    if has(&["dts-hd hra", "dtshd hra", "dts-hd high", "dts hd hra", "hra "]) {
        return ("dts", with("HD HRA"));
    }
    if has(&["dts-hd", "dtshd", "dts hd"]) {
        return ("dts", with("HD"));
    }
    if has(&[" dts", "dts ", " dca "]) {
        return ("dts", with(if has(&["dts-es", "dts es"]) { "ES" } else if has(&["96/24", "96 24"]) { "96/24" } else if has(&["express"]) { "Express" } else { "" }));
    }
    if has(&["truehd", "true hd", " mlp "]) {
        return ("dolby", with("TrueHD"));
    }
    if has(&["dd+", " ddp", "eac3", "e-ac-3", "digital plus"]) {
        return ("dolby", with("Digital Plus"));
    }
    if has(&[" ac3", " ac-3", "dolby digital", " dd "]) {
        return ("dolby", with("Digital"));
    }
    if has(&[" flac"]) {
        return ("flac", ch);
    }
    if has(&[" alac", "apple lossless"]) {
        return ("alac", ch);
    }
    if has(&[" opus"]) {
        return ("opus", ch);
    }
    if has(&[" mp3"]) {
        return ("mp3", ch);
    }
    if has(&["aac", "mp4a"]) {
        return ("aac", with(if has(&["he-aac", "he aac"]) { "HE" } else { "" }));
    }
    if has(&["pcm", " wav "]) {
        return ("", with("PCM"));
    }
    ("", stream.trim().to_string())
}

/// "5.1", "7.1", "Stereo"…, from either a layout or the driver's "N channels".
fn channels(s: &str) -> String {
    let words: Vec<&str> = s.split_whitespace().collect();
    if let Some(layout) = words.iter().find(|w| w.len() == 3 && w.as_bytes()[1] == b'.' && w.chars().all(|c| c.is_ascii_digit() || c == '.')) {
        return if *layout == "2.0" { "Stereo".into() } else { layout.to_string() };
    }
    if words.contains(&"stereo") {
        return "Stereo".into();
    }
    if words.contains(&"mono") {
        return "Mono".into();
    }
    match words.windows(2).find(|w| w[1].starts_with("channel")).and_then(|w| w[0].parse::<u32>().ok()) {
        Some(1) => "Mono".into(),
        Some(2) => "Stereo".into(),
        Some(n @ 3..=8) => format!("{}.1", n - 1),
        _ => String::new(),
    }
}

struct CodecLogo {
    path: SharedString,
    /// The part of the viewbox the mark fills: top, width, height.
    top: f32,
    width: f32,
    height: f32,
    /// How tall it is drawn, in logical pixels.
    size: f32,
    /// A wordmark with no descenders, set on the label's baseline instead of centred.
    baseline: bool,
}

fn codec_logo(app: &AppWindow, id: &str) -> CodecLogo {
    let c = app.global::<Codecs>();
    let lockup = |path: SharedString, width: f32| CodecLogo { path, top: 0.0, width, height: 24.0, size: 30.0, baseline: false };
    match id {
        // The simple-icons DTS mark sits in a 24x24 box with empty space above and below.
        "dolby" => CodecLogo { path: c.get_dolby_box(), top: 0.0, width: 33.03, height: 24.0, size: 20.0, baseline: false },
        "dts" => CodecLogo { path: app.global::<Logos>().get_dts(), top: 7.12, width: 24.0, height: 9.76, size: 17.0, baseline: true },
        "flac" => CodecLogo { size: 18.0, baseline: true, ..lockup(c.get_flac(), 49.63) },
        "opus" => lockup(c.get_opus(), 42.19),
        "alac" => lockup(c.get_alac(), 44.99),
        "mp3" => lockup(c.get_mp3(), 47.89),
        "aac" => lockup(c.get_aac(), 23.91),
        _ => lockup(SharedString::new(), 24.0),
    }
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
        assert_eq!(volume_percent(72, 72), 100);
        assert_eq!(volume_percent(36, 72), 50);
        assert_eq!(volume_percent(80, 72), 100);
        assert_eq!(slide(0, 100, 72), 72);
        assert_eq!(slide(36, 1, 72), 37);
        assert_eq!(slide(36, -50, 72), 0);
        assert_eq!(clock(4328.0), "1:12:08");
        assert_eq!(clock(548.0), "9:08");
        assert_eq!(episode("episode", "S1E4"), "Season 1 · Episode 4");
        assert_eq!(episode("movie", "English - English"), "");
        assert_eq!(split_track("French VF - AAC-LC stereo"), ("French VF".into(), "AAC-LC stereo".into()));
        let badge = |s: &str| { let (l, t) = codec_badge(s); (l, t) };
        assert_eq!(badge("DTS-HD MA 7.1"), ("dts", "HD MA 7.1".to_string()));
        assert_eq!(badge("TrueHD Atmos 7.1"), ("dolby", "TrueHD Atmos 7.1".to_string()));
        assert_eq!(badge("Truehd_Atmos 8 channels"), ("dolby", "TrueHD Atmos 7.1".to_string()));
        assert_eq!(badge("DD+ 5.1"), ("dolby", "Digital Plus 5.1".to_string()));
        assert_eq!(badge("Dolby Digital 5.1"), ("dolby", "Digital 5.1".to_string()));
        assert_eq!(badge("AAC-LC stereo"), ("aac", "Stereo".to_string()));
        assert_eq!(badge("DTS:X 7.1"), ("dts", "X 7.1".to_string()));
        assert_eq!(badge("Pcm_S24Le 2 channels"), ("", "PCM Stereo".to_string()));
        assert_eq!(badge("FLAC 2.0"), ("flac", "Stereo".to_string()));
        assert_eq!(audio_format("English DTS-HD MA 5.1 - DTS-HD MA 5.1(side)"), "DTS-HD MA 5.1");
        assert_eq!(codec_badge(&audio_format("English DTS-HD MA 5.1 - DTS-HD MA 5.1(side)")), ("dts", "HD MA 5.1".to_string()));
        assert_eq!(audio_format(""), "");
    }

    #[test]
    fn temporary_titles_are_ignored() {
        let t0 = Instant::now();
        let mut t = StableTitle::default();
        // Playback starts: the driver flashes the track names first.
        t.feed("m1", "English - English", "English - English", t0);
        assert_eq!(t.shown, "");
        t.feed("m1", "Patton", "English - English", t0);
        assert_eq!(t.shown, "Patton");
        // A chapter name appears for a few seconds, then the title returns.
        t.feed("m1", "Chapter 4", "English - English", t0);
        t.feed("m1", "Chapter 4", "English - English", t0 + Duration::from_secs(4));
        assert_eq!(t.shown, "Patton");
        t.feed("m1", "Patton", "English - English", t0 + Duration::from_secs(5));
        // A change that lasts is real.
        t.feed("m1", "Patton (Extended)", "", t0 + Duration::from_secs(10));
        t.feed("m1", "Patton (Extended)", "", t0 + Duration::from_secs(18));
        assert_eq!(t.shown, "Patton (Extended)");
        // Another film takes over at once.
        t.feed("m2", "Alien", "", t0 + Duration::from_secs(20));
        assert_eq!(t.shown, "Alien");
    }
}
