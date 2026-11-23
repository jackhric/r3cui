//! Preview captures with demo data. `--shots <dir>` saves a PNG per screen; `--record <file.mp4>`
//! plays a scripted session and pipes frames to ffmpeg. With `--frame <photo>` both show the UI
//! inside the remote with its keys lit.

use crate::backlight;
use crate::device::{self, Device};
use crate::{apply, apply_demo_settings, parse_color, set_accent, AppWindow, Model, UiAction};
use anyhow::Context;
use slint::{ComponentHandle, Model as _};
use std::cell::RefCell;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

/// Lit zones and their colour for what the window is showing.
fn keys(app: &AppWindow) -> (&'static [u8], [u8; 3]) {
    let (state, accent) = backlight::observe(app);
    (backlight::zones(state), backlight::colour(state, accent))
}

type Step = (&'static str, fn(&AppWindow, &mut Model));

/// D-pad presses that type a plausible password, wandering over all four rows.
const TYPING: &[&str] = &["ok", "right", "ok", "up", "ok", "left", "left", "ok", "down", "down", "ok", "right", "right", "ok", "up", "ok", "up", "right", "ok", "ok"];

const STEPS: &[Step] = &[
    ("01-receiver", |_, _| {}),
    ("02-inputs", |app, _| app.invoke_go(1, app.get_source_index())),
    ("03-kodi-playing", |app, m| {
        app.invoke_go(0, 0);
        m.source = "KODI".into();
    }),
    ("04-kodi-browsing", |_, m| m.title.clear()),
    ("05-volume", |app, m| {
        m.title = Model::demo().title;
        m.volume = 64;
        app.set_volume_overlay(true);
    }),
    ("06-muted", |_, m| m.muted = true),
    ("07-menu", |app, m| {
        app.set_volume_overlay(false);
        m.muted = false;
        m.volume = 62;
        app.invoke_go(2, 1);
    }),
    ("08-menu-display", |app, _| app.invoke_go(3, 0)),
    ("09-menu-power", |app, _| app.invoke_go(4, 0)),
    ("10-accent-amber-kodi", |app, _| {
        set_accent(app, parse_color("#FFB454").unwrap());
        app.invoke_go(0, 0);
    }),
    ("11-accent-amber-receiver", |_, m| m.source = "PLAYSTATION 5".into()),
    ("12-accent-violet-volume", |app, m| {
        set_accent(app, parse_color("#A78BFA").unwrap());
        m.source = "KODI".into();
        app.set_volume_overlay(true);
    }),
    ("13-receiver-off", |app, m| {
        set_accent(app, parse_color("#E8343A").unwrap());
        app.set_volume_overlay(false);
        m.receiver_on = false;
    }),
    ("14-wifi", |app, m| {
        m.receiver_on = true;
        app.invoke_go(5, 1);
    }),
    ("15-keyboard", |app, _| {
        app.set_wifi_target(app.get_wifi_rows().row_data(1).unwrap_or_default());
        app.invoke_kb_reset();
        app.invoke_go(6, 0);
        for k in TYPING {
            app.invoke_kb_nav((*k).into());
        }
    }),
    ("16-joining", |app, _| app.set_join_state(1)),
    ("17-join-failed", |app, _| {
        app.set_join_state(2);
        app.invoke_go(8, 0);
    }),
    ("18-status", |app, _| {
        app.set_join_state(0);
        app.invoke_go(7, 0);
    }),
    ("19-status-ha-down", |_, m| m.demo_ha_down = true),
    ("20-home-warning", |app, _| app.invoke_go(0, 0)),
];

pub fn run(app: AppWindow, dir: PathBuf, mut frame: Option<Device>) -> anyhow::Result<()> {
    std::fs::create_dir_all(&dir)?;
    apply_demo_settings(&app);
    app.set_connected(true);
    let model = Rc::new(RefCell::new(Model::demo()));
    apply(&app, &model.borrow());

    let timer = slint::Timer::default();
    let weak = app.as_weak();
    let mut next = 0usize;
    // Long enough for the page transition to settle before the frame is grabbed.
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(700), move || {
        let Some(app) = weak.upgrade() else { return };
        if next > 0 {
            let path = dir.join(format!("{}.png", STEPS[next - 1].0));
            let saved = app.window().take_snapshot().map_err(anyhow::Error::from).and_then(|buf| {
                match &mut frame {
                    Some(device) => device
                        .compose(buf.as_bytes(), (buf.width(), buf.height()), keys(&app).0, keys(&app).1, f32::MAX)
                        .save(&path)?,
                    None => image::save_buffer(&path, buf.as_bytes(), buf.width(), buf.height(), image::ColorType::Rgba8)?,
                }
                Ok(())
            });
            println!("{} {saved:?}", path.display());
        }
        match STEPS.get(next) {
            Some((_, step)) => {
                let mut m = model.borrow_mut();
                step(&app, &mut m);
                let updated = m.clone().with_demo_health();
                *m = updated;
                apply(&app, &m);
                next += 1;
            }
            None => {
                let _ = slint::quit_event_loop();
            }
        }
    });

    app.run()?;
    Ok(())
}

/// `--snap <png>`: one picture of whatever the app is showing a few seconds after launch, then quit.
/// Works in live mode, so it shows real data from the core.
pub fn snap(app: &AppWindow, path: PathBuf, mut frame: Option<Device>) -> slint::Timer {
    let weak = app.as_weak();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::SingleShot, Duration::from_secs(5), move || {
        let Some(app) = weak.upgrade() else { return };
        let saved = app.window().take_snapshot().map_err(anyhow::Error::from).and_then(|buf| {
            match &mut frame {
                Some(device) => device
                    .compose(buf.as_bytes(), (buf.width(), buf.height()), keys(&app).0, keys(&app).1, f32::MAX)
                    .save(&path)?,
                None => image::save_buffer(&path, buf.as_bytes(), buf.width(), buf.height(), image::ColorType::Rgba8)?,
            }
            Ok(())
        });
        println!("{} {saved:?}", path.display());
        let _ = slint::quit_event_loop();
    });
    timer
}

type Tx = UnboundedSender<UiAction>;
type Act = Box<dyn Fn(&AppWindow, &Tx)>;

fn key(k: &'static str) -> Act {
    Box::new(move |_, tx| {
        let _ = tx.send(UiAction::Key(k.into()));
    })
}

fn ui(f: fn(&AppWindow)) -> Act {
    Box::new(move |app, _| f(app))
}

/// Sleep and wake, on the Kodi screen and then on the receiver screen.
fn power_script() -> Vec<(u64, Act)> {
    let source = |name: &'static str| -> Act {
        Box::new(move |_, tx| {
            let _ = tx.send(UiAction::SelectSource(name.into()));
        })
    };
    vec![
        (300, source("KODI")),
        (2200, key("z")),
        (2400, key("z")),
        (2800, key("z")),
        (2200, source("PLAYSTATION 5")),
        (300, key("z")),
        (2800, key("z")),
        (2400, ui(|_| {})),
    ]
}

/// The physical power key, through real key events: a short tap (nothing happens), then a hold
/// (receiver off), from the menu to show it works anywhere, then a hold to turn it back on.
fn hold_script() -> Vec<(u64, Act)> {
    use slint::platform::WindowEvent;
    let down: fn(&AppWindow) = |a| a.window().dispatch_event(WindowEvent::KeyPressed { text: "p".into() });
    let up: fn(&AppWindow) = |a| a.window().dispatch_event(WindowEvent::KeyReleased { text: "p".into() });
    vec![
        (1500, ui(down)),
        (450, ui(up)),
        (1800, ui(|a| a.invoke_go(2, 0))),
        (1300, ui(down)),
        (1500, ui(up)),
        (1400, ui(|a| a.invoke_back())),
        (2200, ui(down)),
        (1500, ui(up)),
        (2200, ui(|_| {})),
    ]
}

/// Menu → Wi-Fi → pick a network → type its password with the d-pad → a failed join, a retry
/// that works, then the status page with Home Assistant down and the warning on the home screen.
fn wifi_script() -> Vec<(u64, Act)> {
    let mut steps: Vec<(u64, Act)> = vec![
        (1200, ui(|a| a.invoke_go(2, 0))),
        (800, ui(|a| a.invoke_activate(0))),
        (2400, ui(|a| a.invoke_move(1))),
        (700, ui(|a| a.invoke_activate(1))),
        (900, ui(|_| {})),
    ];
    for k in TYPING {
        steps.push((260, Box::new(move |a: &AppWindow, _: &Tx| a.invoke_kb_nav((*k).into()))));
    }
    steps.extend([
        (700, ui(|a| a.invoke_kb_tap(30))),
        (3400, ui(|a| a.invoke_activate(0))),
        (1100, ui(|a| a.invoke_kb_tap(30))),
        (3600, ui(|a| a.invoke_back())),
        (700, ui(|a| a.invoke_activate(4))),
        (1800, key("x")),
        (2200, ui(|a| a.invoke_back())),
        (600, ui(|a| a.invoke_back())),
        (2600, ui(|_| {})),
    ]);
    steps
}

/// (pause before the step in ms, what happens)
fn script(name: Option<&str>) -> Vec<(u64, Act)> {
    if name == Some("wifi") {
        return wifi_script();
    }
    if name == Some("power") {
        return power_script();
    }
    if name == Some("hold") {
        return hold_script();
    }
    vec![
        // Receiver: nudge the volume.
        (1400, key("+")),
        (320, key("+")),
        (320, key("+")),
        // Open the input list, walk up to KODI, select it.
        (1300, ui(|a| a.invoke_go(1, a.get_source_index()))),
        (800, ui(|a| a.invoke_move(-1))),
        (380, ui(|a| a.invoke_move(-1))),
        (700, ui(|a| a.invoke_activate(a.get_focus_index()))),
        // Kodi is playing: volume overlay, mute, unmute.
        (2400, key("+")),
        (320, key("+")),
        (2600, key("0")),
        (1500, key("0")),
        // Stop (browsing), then play again.
        (2600, key("s")),
        (2000, key(" ")),
        // Menu → Display → brightness → back → Power → back out.
        (2000, ui(|a| a.invoke_go(2, 0))),
        (700, ui(|a| a.invoke_move(1))),
        (600, ui(|a| a.invoke_activate(1))),
        (900, ui(|a| a.set_brightness(70))),
        (600, ui(|a| a.set_brightness(45))),
        (700, ui(|a| a.set_brightness(90))),
        (900, ui(|a| a.invoke_back())),
        (600, ui(|a| a.invoke_move(1))),
        (320, ui(|a| a.invoke_move(1))),
        (600, ui(|a| a.invoke_activate(3))),
        (1200, ui(|a| a.invoke_back())),
        (700, ui(|a| a.invoke_back())),
        // The interface colour changes at runtime; the keys follow.
        (1400, ui(|a| set_accent(a, parse_color("#FFB454").unwrap()))),
        (1700, ui(|a| set_accent(a, parse_color("#A78BFA").unwrap()))),
        (1700, ui(|a| set_accent(a, parse_color("#E8343A").unwrap()))),
        (1600, ui(|_| {})),
    ]
}

/// Starts the recording; the returned timer must be kept alive until the event loop ends.
pub fn record(app: &AppWindow, tx: Tx, mut device: Device, out: PathBuf, which: Option<&str>) -> anyhow::Result<slint::Timer> {
    let mut ffmpeg = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgb24"])
        .args(["-s", &format!("{}x{}", device::WIDTH, device::HEIGHT)])
        .args(["-use_wallclock_as_timestamps", "1", "-i", "-"])
        .args(["-vf", "fps=30", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "20", "-movflags", "+faststart"])
        .arg(&out)
        .stdin(Stdio::piped())
        .spawn()
        .context("starting ffmpeg")?;
    let mut pipe = Some(ffmpeg.stdin.take().context("ffmpeg stdin")?);

    let mut steps = script(which).into_iter();
    let mut pending = steps.next();
    let mut since = Instant::now();
    let mut last_frame = Instant::now();
    let mut ffmpeg = Some(ffmpeg);
    let weak = app.as_weak();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(33), move || {
        let Some(app) = weak.upgrade() else { return };
        if pending.as_ref().is_some_and(|(wait, _)| since.elapsed() >= Duration::from_millis(*wait)) {
            if let Some((_, act)) = pending.take() {
                act(&app, &tx);
            }
            since = Instant::now();
            pending = steps.next();
            if pending.is_none() {
                // Closing the pipe is what tells ffmpeg the stream has ended.
                drop(pipe.take());
                if let Some(mut child) = ffmpeg.take() {
                    println!("{} {:?}", out.display(), child.wait());
                }
                let _ = slint::quit_event_loop();
                return;
            }
        }
        if let Ok(buf) = app.window().take_snapshot() {
            let dt = last_frame.elapsed().as_secs_f32();
            last_frame = Instant::now();
            let frame = device.compose(buf.as_bytes(), (buf.width(), buf.height()), keys(&app).0, keys(&app).1, dt);
            if let Some(pipe) = &mut pipe {
                let _ = pipe.write_all(frame.as_raw());
            }
        }
    });
    Ok(timer)
}
