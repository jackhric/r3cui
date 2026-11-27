//! remote-ui: a theater remote. Global control (volume, inputs) and each input's remote and UI slots are filled by
//! drivers set up in `modules.json`; the remote's own state comes from the core.
//! The tokio runtime runs on its own thread and pushes a `Model` into the Slint event loop via
//! `invoke_from_event_loop`; UI callbacks flow back over a channel. See docs/plan.md.

mod app;
mod art;
mod hardware;
mod input;
#[cfg(feature = "preview")]
mod preview;
mod remote;
mod remote_control;
mod view;

use crate::app::{UiAction, settings};
use crate::app::session::{Frontend, RECONNECT_DELAY, run};
use crate::art::grain;
use crate::hardware::{backlight, feedback};
use crate::input::{key_action, keys, wire_keyboard};
use crate::view::{parse_color, set_accent, set_status};
use crate::view::screen::{Screen, page_by_name};
use crate::view::idle::{animate_lava, poke, rest_on_charger};
use crate::view::stage::capture;
#[cfg(feature = "device")]
use crate::hardware::frames;
#[cfg(feature = "device")]
use crate::input::evdev;
#[cfg(feature = "preview")]
use crate::preview::device;
#[cfg(feature = "preview")]
use crate::preview::shots;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing_subscriber::EnvFilter;

slint::include_modules!();

const BACKLIGHT_POLL: Duration = Duration::from_millis(150);
/// Pages blur the screen behind them, which needs the window to be showing first.
const PAGE_FLAG_DELAY: Duration = Duration::from_millis(1500);
#[cfg(feature = "preview")]
const PRESS_FLAG_DELAY: Duration = Duration::from_millis(4000);

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
    let key_chars = app.global::<KeyChars>();
    key_chars.set_home(keys::HOME.into());
    key_chars.set_input(keys::INPUT.into());
    key_chars.set_menu(keys::MENU.into());
    key_chars.set_power(keys::POWER.into());
    app.global::<Fx>().set_grain(grain());
    #[cfg(feature = "device")]
    frames::install(&app);
    // UC_ACCENT pins one colour; without it the colour follows the input (see `apply`).
    if let Some(accent) = std::env::var("UC_ACCENT").ok().as_deref().and_then(parse_color) {
        set_accent(&app, accent);
    }

    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1));
    #[cfg(feature = "preview")]
    let frame = arg("--frame").map(|p| device::Device::load(p.as_ref())).transpose()?;
    let (act_tx, mut act_rx) = mpsc::unbounded_channel();
    #[cfg(feature = "device")]
    evdev::start(&app, act_tx.clone());
    wire_keyboard(&app, act_tx.clone());
    app.global::<Fx>().on_capture({
        let weak = app.as_weak();
        move |which| {
            if let Some(app) = weak.upgrade() {
                capture(&app, which);
            }
        }
    });
    #[cfg(feature = "preview")]
    if let Some(dir) = arg("--shots") {
        return shots::run(app, dir.into(), frame);
    }
    #[cfg(feature = "preview")]
    let record = arg("--record");
    // Recordings play the demo unless `--live` asks for the core's real state.
    #[cfg(feature = "demo")]
    let demo = {
        let live = args.iter().any(|a| a == "--live");
        let demo = (arg("--record").is_some() && !live) || args.iter().any(|a| a == "--demo") || std::env::var_os("UC_DEMO").is_some();
        if demo {
            app::demo::apply_demo_settings(&app);
        }
        demo
    };

    let weak = app.as_weak();
    let send = |f: fn(String) -> UiAction| {
        let tx = act_tx.clone();
        move |s: SharedString| {
            let _ = tx.send(f(s.into()));
        }
    };
    app.global::<Control>().on_play_pause({
        let tx = act_tx.clone();
        move || {
            let _ = tx.send(UiAction::PlayPause);
        }
    });
    app.global::<Control>().on_key({
        let tx = act_tx.clone();
        move |text| {
            if let Some(action) = key_action(&text) {
                let _ = tx.send(action);
            }
        }
    });
    app.global::<Control>().on_select_input(send(UiAction::SelectInput));
    {
        use crate::app::Settings;
        let tx = act_tx.clone();
        app.global::<Cfg>().on_open(move || drop(tx.send(UiAction::Integrations(Settings::Open))));
        let tx = act_tx.clone();
        app.global::<Cfg>().on_open_for_inputs(move || drop(tx.send(UiAction::Integrations(Settings::OpenForInputs))));
        let tx = act_tx.clone();
        app.global::<Cfg>().on_pick(move |i| drop(tx.send(UiAction::Integrations(Settings::Pick(i.max(0) as usize)))));
        let tx = act_tx.clone();
        app.global::<Cfg>().on_back(move || drop(tx.send(UiAction::Integrations(Settings::Back))));
    }
    {
        use crate::app::software::Action;
        let software = app.global::<Software>();
        let act = |action: fn() -> Action| {
            let tx = act_tx.clone();
            move || drop(tx.send(UiAction::Software(action())))
        };
        software.on_opened(act(|| Action::Opened));
        software.on_check_r3(act(|| Action::CheckR3));
        software.on_install_r3(act(|| Action::InstallR3));
        software.on_check_fw(act(|| Action::CheckFirmware));
        software.on_install_fw(act(|| Action::InstallFirmware));
        software.on_open_web(act(|| Action::OpenWeb));
        software.on_new_pin(act(|| Action::NewPin));
        software.on_web_off(act(|| Action::WebOff));
        software.on_uninstall(act(|| Action::Uninstall));
        software.on_install_voice(act(|| Action::InstallVoice));
    }
    app.global::<Control>().on_power_action({
        let tx = act_tx.clone();
        move |action| drop(tx.send(UiAction::Power(action)))
    });
    app.global::<Fx>().on_haptic(feedback::ui_event);
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
            app.global::<Lab>().set_title(lab.title().into());
            app.global::<Lab>().set_rows(ModelRc::new(VecModel::from(rows)));
        }
    };
    show_lab();
    app.global::<Lab>().on_pick({
        let (lab, show_lab) = (lab.clone(), show_lab.clone());
        move |i| {
            let moved = lab.borrow_mut().pick(i as usize);
            show_lab();
            moved
        }
    });
    app.global::<Lab>().on_back(move || {
        let focus = lab.borrow_mut().back();
        show_lab();
        focus.map_or(-1, |f| f as i32)
    });
    app.global::<Wifi>().on_scan({
        let tx = act_tx.clone();
        move || {
            let _ = tx.send(UiAction::WifiScan);
        }
    });
    app.global::<Wifi>().on_band({
        let tx = act_tx.clone();
        move || {
            let _ = tx.send(UiAction::WifiBand);
        }
    });
    app.global::<Wifi>().on_join({
        let tx = act_tx.clone();
        move |ssid, hex, password| {
            let _ = tx.send(UiAction::WifiJoin { ssid: ssid.into(), hex: hex.into(), password: password.into() });
        }
    });
    app.global::<Control>().on_seek({
        let tx = act_tx.clone();
        move |by| {
            let _ = tx.send(UiAction::Seek(by));
        }
    });
    app.global::<Tracks>().on_open({
        let tx = act_tx.clone();
        move |kind| {
            let _ = tx.send(UiAction::OpenPicker(kind));
        }
    });
    app.global::<Tracks>().on_pick({
        let tx = act_tx.clone();
        move |kind, i| {
            let _ = tx.send(UiAction::PickTrack(kind, i.max(0) as usize));
        }
    });
    app.global::<Sys>().on_activity(poke);
    app.global::<Prefs>().on_changed({
        let (tx, weak) = (act_tx.clone(), app.as_weak());
        move || {
            if let Some(app) = weak.upgrade() {
                let _ = tx.send(UiAction::Display { brightness: app.global::<Prefs>().get_brightness(), auto: app.global::<Prefs>().get_auto_brightness() });
            }
        }
    });
    app.global::<Control>().on_power_held({
        let tx = act_tx.clone();
        move || {
            let _ = tx.send(UiAction::PowerHeld);
        }
    });
    let voice_key = |down| {
        let tx = act_tx.clone();
        move || drop(tx.send(UiAction::VoiceKey { down }))
    };
    app.global::<Voice>().on_press(voice_key(true));
    app.global::<Voice>().on_release(voice_key(false));
    app.global::<Voice>().on_dismiss({
        let tx = act_tx.clone();
        move || drop(tx.send(UiAction::VoiceDismiss))
    });

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async move {
            #[cfg(feature = "demo")]
            if demo {
                let mut front = Frontend::demo(weak.clone());
                app::demo::run(&weak, &mut act_rx, &mut front).await;
                return;
            }
            let mut front = Frontend::start(weak.clone());
            loop {
                match run(&weak, &mut act_rx, &mut front).await {
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

    if let Some(page) = arg("--page").and_then(|p| page_by_name(p)) {
        let weak = app.as_weak();
        slint::Timer::single_shot(PAGE_FLAG_DELAY, move || {
            if let Some(app) = weak.upgrade() {
                app.go_to(page, 0);
            }
        });
    }
    // `--press <key>`: one key press shortly before `--snap` takes its picture.
    #[cfg(feature = "preview")]
    if let Some(key) = arg("--press").cloned() {
        let tx = act_tx.clone();
        slint::Timer::single_shot(PRESS_FLAG_DELAY, move || {
            if let Some(action) = key_action(&key) {
                let _ = tx.send(action);
            }
        });
    }
    #[cfg(feature = "preview")]
    let _steps = match arg("--steps") {
        Some(script) => Some(preview::steps::run(&app, std::path::Path::new(script), arg("--out").map_or_else(|| "steps".into(), Into::into))?),
        None => None,
    };
    #[cfg(feature = "preview")]
    let _snap = arg("--snap").map(|out| shots::snap(&app, out.into(), device::Device::load_opt(arg("--frame"))));
    // The window is the source of truth for what is on screen, so the key backlight follows it
    // from here; the core is only told when the answer changes.
    let _backlight = {
        let (weak, tx) = (app.as_weak(), act_tx.clone());
        let mut last = None;
        let timer = slint::Timer::default();
        timer.start(slint::TimerMode::Repeated, BACKLIGHT_POLL, move || {
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
    let _aurora = crate::view::voice::animate_aurora(&app);
    crate::view::voice::bench_aurora();
    let remote = remote_control::start(app.as_weak(), act_tx.clone());
    app.global::<RemoteControl>().on_toggle({
        let remote = remote.clone();
        move || remote.toggle()
    });
    app.global::<RemoteControl>().on_new_key(move || remote.new_key());
    #[cfg(feature = "preview")]
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
