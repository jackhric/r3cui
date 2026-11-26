//! Preview captures with demo data. `--shots <dir>` saves a PNG per screen; `--record <file.mp4>`
//! plays a scripted session and pipes frames to ffmpeg. With `--frame <photo>` both show the UI
//! inside the remote with its keys lit.

use crate::hardware::backlight;
use crate::view::screen::{Capture, JoinState, LibMode, Page, Picker, Screen};
use crate::preview::device::{self, Device};
use crate::{view::apply, app::demo::apply_demo_settings, AppWindow, app::model::Model, app::UiAction};
use anyhow::Context;
use slint::{ComponentHandle, Model as _};
use std::cell::RefCell;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

/// What the keys show for what the window is showing.
fn keys(app: &AppWindow) -> backlight::Keys {
    let (state, accent) = backlight::observe(app);
    backlight::keys_for(state, accent)
}

type Step = (&'static str, fn(&AppWindow, &mut Model));

/// D-pad presses that type a plausible password, wandering over all four rows.
const TYPING: &[&str] = &["ok", "right", "ok", "up", "ok", "left", "left", "ok", "down", "down", "ok", "right", "right", "ok", "up", "ok", "up", "right", "ok", "ok"];

const STEPS: &[Step] = &[
    ("01-receiver", |_, _| {}),
    ("02-inputs", |app, _| app.go_to(Page::Inputs, app.get_source_index())),
    ("03-kodi-playing", |app, m| {
        app.go_to(Page::Home, 0);
        m.source = "KODI".into();
    }),
    ("03b-audio", |app, m| {
        app.show_picker(Picker::Audio);
        app.go_to(Page::Tracks, 0);
        let (rows, current) = crate::app::demo::demo_tracks(m, Picker::Audio, 1);
        crate::view::show_tracks(&app.as_weak(), rows, current);
    }),
    ("03c-subtitles", |app, m| {
        app.show_picker(Picker::Subtitles);
        app.go_to(Page::Tracks, 0);
        let (rows, current) = crate::app::demo::demo_tracks(m, Picker::Subtitles, 1);
        crate::view::show_tracks(&app.as_weak(), rows, current);
    }),
    ("03d-paused", |app, m| {
        app.go_to(Page::Home, 0);
        m.paused = true;
    }),
    ("04-kodi-browsing", |_, m| {
        m.paused = false;
        m.title.clear();
    }),
    ("05-volume", |app, m| {
        m.title = Model::demo().title;
        m.volume = 64;
        // Blur Now playing, not the stage it replaces.
        after_swap(app, |app| {
            crate::view::stage::capture(app, Capture::World);
            app.set_volume_overlay(true);
        });
    }),
    ("06-muted", |_, m| m.muted = true),
    ("07-menu", |app, m| {
        app.set_volume_overlay(false);
        m.muted = false;
        m.volume = 62;
        app.go_to(Page::Menu, 1);
    }),
    ("07b-menu-playstation", |_, m| m.source = "PLAYSTATION 5".into()),
    ("07c-menu-xbox", |_, m| m.source = "XBOX 360".into()),
    ("07d-menu-kodi", |_, m| m.source = "KODI".into()),
    ("08-menu-display", |app, _| app.go_to(Page::Display, 0)),
    ("09-menu-power", |app, _| app.go_to(Page::Power, 0)),
    ("10-playstation", |app, m| {
        app.go_to(Page::Home, 0);
        m.source = "PLAYSTATION 5".into();
    }),
    ("11-xbox", |_, m| m.source = "XBOX 360".into()),
    ("12-android-tv", |_, m| m.source = "ANDROID TV".into()),
    ("13-receiver-off", |_, m| {
        m.source = "KODI".into();
        m.receiver_on = false;
    }),
    ("14-wifi", |app, m| {
        m.receiver_on = true;
        app.go_to(Page::Wifi, 1);
    }),
    ("15-keyboard", |app, _| {
        app.set_wifi_target(app.get_wifi_rows().row_data(1).unwrap_or_default());
        app.invoke_kb_reset();
        app.go_to(Page::Password, 0);
        for k in TYPING {
            app.invoke_kb_nav((*k).into());
        }
    }),
    ("16-joining", |app, _| app.show_join(JoinState::Joining)),
    ("17-join-failed", |app, _| {
        app.show_join(JoinState::Failed);
        app.go_to(Page::JoinFailed, 0);
    }),
    ("18-status", |app, _| {
        app.show_join(JoinState::Idle);
        app.go_to(Page::Status, 0);
    }),
    ("19-status-ha-down", |_, m| m.demo_ha_down = true),
    ("20-home-warning", |app, _| app.go_to(Page::Home, 0)),
    ("21-charging", |_, m| {
        m.demo_ha_down = false;
        m.charging = true;
    }),
    ("22-charging-home", |app, _| app.set_charge_card(false)),
    ("23-charging-kodi", |_, m| {
        m.source = "KODI".into();
        m.title = Model::demo().title;
    }),
    ("30-codec-00", |app, m| {
        app.go_to(Page::Home, 0);
        m.charging = false;
        m.source = "KODI".into();
        m.title = Model::demo().title;
        m.media_format = "TrueHD Atmos 7.1".into();
    }),
    ("30-codec-01", |app, m| {
        app.go_to(Page::Home, 0);
        m.charging = false;
        m.source = "KODI".into();
        m.title = Model::demo().title;
        m.media_format = "DD+ Atmos 5.1".into();
    }),
    ("30-codec-02", |app, m| {
        app.go_to(Page::Home, 0);
        m.charging = false;
        m.source = "KODI".into();
        m.title = Model::demo().title;
        m.media_format = "DTS:X 7.1".into();
    }),
    ("30-codec-03", |app, m| {
        app.go_to(Page::Home, 0);
        m.charging = false;
        m.source = "KODI".into();
        m.title = Model::demo().title;
        m.media_format = "DTS-HD MA 7.1".into();
    }),
    ("30-codec-04", |app, m| {
        app.go_to(Page::Home, 0);
        m.charging = false;
        m.source = "KODI".into();
        m.title = Model::demo().title;
        m.media_format = "DTS-HD HRA 5.1".into();
    }),
    ("30-codec-05", |app, m| {
        app.go_to(Page::Home, 0);
        m.charging = false;
        m.source = "KODI".into();
        m.title = Model::demo().title;
        m.media_format = "DTS 5.1".into();
    }),
    ("30-codec-06", |app, m| {
        app.go_to(Page::Home, 0);
        m.charging = false;
        m.source = "KODI".into();
        m.title = Model::demo().title;
        m.media_format = "TrueHD 7.1".into();
    }),
    ("30-codec-07", |app, m| {
        app.go_to(Page::Home, 0);
        m.charging = false;
        m.source = "KODI".into();
        m.title = Model::demo().title;
        m.media_format = "DD+ 5.1".into();
    }),
    ("30-codec-08", |app, m| {
        app.go_to(Page::Home, 0);
        m.charging = false;
        m.source = "KODI".into();
        m.title = Model::demo().title;
        m.media_format = "Dolby Digital 5.1".into();
    }),
    ("30-codec-09", |app, m| {
        app.go_to(Page::Home, 0);
        m.charging = false;
        m.source = "KODI".into();
        m.title = Model::demo().title;
        m.media_format = "FLAC 2.0".into();
    }),
    ("30-codec-10", |app, m| {
        app.go_to(Page::Home, 0);
        m.charging = false;
        m.source = "KODI".into();
        m.title = Model::demo().title;
        m.media_format = "Alac 2 channels".into();
    }),
    ("30-codec-11", |app, m| {
        app.go_to(Page::Home, 0);
        m.charging = false;
        m.source = "KODI".into();
        m.title = Model::demo().title;
        m.media_format = "Opus 5.1".into();
    }),
    ("30-codec-12", |app, m| {
        app.go_to(Page::Home, 0);
        m.charging = false;
        m.source = "KODI".into();
        m.title = Model::demo().title;
        m.media_format = "MP3 stereo".into();
    }),
    ("30-codec-13", |app, m| {
        app.go_to(Page::Home, 0);
        m.charging = false;
        m.source = "KODI".into();
        m.title = Model::demo().title;
        m.media_format = "AAC-LC stereo".into();
    }),
    ("30-codec-14", |app, m| {
        app.go_to(Page::Home, 0);
        m.charging = false;
        m.source = "KODI".into();
        m.title = Model::demo().title;
        m.media_format = "Pcm_S24Le 2 channels".into();
    }),
];

/// Run `f` once the stage swap under way has finished.
fn after_swap(app: &AppWindow, f: impl FnOnce(&AppWindow) + 'static) {
    let weak = app.as_weak();
    slint::Timer::single_shot(Duration::from_millis(300), move || {
        let Some(app) = weak.upgrade() else { return };
        if app.get_swapping() {
            after_swap(&app, f);
        } else {
            f(&app);
        }
    });
}

pub fn run(app: AppWindow, dir: PathBuf, mut frame: Option<Device>) -> anyhow::Result<()> {
    std::fs::create_dir_all(&dir)?;
    apply_demo_settings(&app);
    app.set_connected(true);
    let model = Rc::new(RefCell::new(Model::demo()));
    apply(&app, &model.borrow());
    let _lava = crate::view::idle::animate_lava(&app);

    let timer = slint::Timer::default();
    let weak = app.as_weak();
    let mut next = 0usize;
    let mut since = Instant::now();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(100), move || {
        let Some(app) = weak.upgrade() else { return };
        // A stage swap can outlast its nominal 620 ms while the blur pictures are built, so wait
        // for it to end and then for the page slide and fades to settle.
        if app.get_swapping() {
            since = Instant::now();
        }
        if since.elapsed() < Duration::from_millis(1500) {
            return;
        }
        since = Instant::now();
        if next > 0 {
            let path = dir.join(format!("{}.png", STEPS[next - 1].0));
            let saved = app.window().take_snapshot().map_err(anyhow::Error::from).and_then(|buf| {
                match &mut frame {
                    Some(device) => device
                        .compose(buf.as_bytes(), (buf.width(), buf.height()), keys(&app), f32::MAX)
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
                    .compose(buf.as_bytes(), (buf.width(), buf.height()), keys(&app), f32::MAX)
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
        if let Some(action) = crate::input::key_action(k) {
            let _ = tx.send(action);
        }
    })
}

/// Pause or play on, with the remote's own play/pause key, printing when, for the edit. Pausing also seeks back 3 s,
/// which has Kodi read the subtitles again from the start of the line on show.
fn kodi_hold(hold: bool) -> Act {
    Box::new(move |_, tx| {
        println!("record {} {:.3}", if hold { "hold" } else { "release" }, unix_now());
        let _ = tx.send(UiAction::Button(crate::input::Button::Play));
        if hold {
            let tx = tx.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(600));
                let _ = tx.send(UiAction::Seek(-3));
            });
        }
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
        (1800, ui(|a| a.go_to(Page::Menu, 0))),
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
        (1200, ui(|a| a.go_to(Page::Menu, 0))),
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

/// The home key through real key events, on the Kodi screen: a tap (Kodi's own home, nothing
/// changes here), then a hold that opens the input list, then a tap that returns home.
fn home_script() -> Vec<(u64, Act)> {
    use slint::platform::WindowEvent;
    let down: fn(&AppWindow) = |a| a.window().dispatch_event(WindowEvent::KeyPressed { text: "h".into() });
    let up: fn(&AppWindow) = |a| a.window().dispatch_event(WindowEvent::KeyReleased { text: "h".into() });
    let kodi: Act = Box::new(|_, tx| {
        let _ = tx.send(UiAction::SelectSource("KODI".into()));
    });
    vec![(300, kodi), (1800, ui(down)), (120, ui(up)), (1500, ui(down)), (900, ui(up)), (2200, ui(down)), (120, ui(up)), (1800, ui(|_| {}))]
}

/// A finger on the strip below the screen, run through the real gesture logic: `path` is
/// positions 0..1 sampled every 30 ms, then the finger lifts.
fn swipe(path: Vec<f32>) -> Act {
    Box::new(move |_, tx| {
        let (tx, path) = (tx.clone(), path.clone());
        std::thread::spawn(move || {
            // One strip for every swipe and tap, as on the remote, so two taps in a row make a double tap.
            static STRIP: std::sync::Mutex<Option<crate::input::slider::Slider>> = std::sync::Mutex::new(None);
            let t0 = Instant::now();
            let send = |touch, pos, at: Instant| {
                let mut strip = STRIP.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(g) = strip.get_or_insert_with(Default::default).feed(touch, pos, at) {
                    let _ = tx.send(UiAction::Slider(g));
                }
            };
            for (i, pos) in path.iter().enumerate() {
                std::thread::sleep(Duration::from_millis(30));
                send(true, *pos, t0 + Duration::from_millis(30 * i as u64));
            }
            send(false, *path.last().unwrap_or(&0.0), t0 + Duration::from_millis(30 * path.len() as u64));
        });
    })
}

fn tap_strip() -> Act {
    swipe(vec![0.5, 0.5])
}

fn press(text: &'static str) -> Act {
    Box::new(move |a, _| {
        use slint::platform::WindowEvent;
        a.window().dispatch_event(WindowEvent::KeyPressed { text: text.into() });
        a.window().dispatch_event(WindowEvent::KeyReleased { text: text.into() });
    })
}

/// Kodi playing: skip back and forward, pause, the slider for volume and a double tap to mute,
/// then the audio and subtitle pickers.
fn media_script() -> Vec<(u64, Act)> {
    let kodi: Act = Box::new(|_, tx| {
        let _ = tx.send(UiAction::SelectSource("KODI".into()));
    });
    let tap = |x: f32, y: f32| -> Act {
        Box::new(move |a, _| {
            use slint::platform::{PointerEventButton, WindowEvent};
            let position = slint::LogicalPosition::new(x, y);
            let button = PointerEventButton::Left;
            a.window().dispatch_event(WindowEvent::PointerMoved { position });
            a.window().dispatch_event(WindowEvent::PointerPressed { position, button });
            a.window().dispatch_event(WindowEvent::PointerReleased { position, button });
            a.window().dispatch_event(WindowEvent::PointerExited);
        })
    };
    vec![
        (300, kodi),
        (1800, tap(116.0, 580.0)),
        (700, tap(116.0, 580.0)),
        (900, tap(364.0, 580.0)),
        (1000, tap(240.0, 584.0)),
        (1400, tap(240.0, 584.0)),
        // Slide right for louder, then back down a little.
        (1200, swipe((0..24).map(|i| 0.3 + i as f32 * 0.012).collect())),
        (1400, swipe((0..14).map(|i| 0.6 - i as f32 * 0.01).collect())),
        // Double tap: mute, and again to unmute.
        (1600, tap_strip()),
        (180, tap_strip()),
        (1800, tap_strip()),
        (180, tap_strip()),
        (2000, tap(348.0, 746.0)),
        (1300, ui(|a| a.invoke_move(1))),
        (500, press("\n")),
        (1500, tap(132.0, 746.0)),
        (1300, ui(|a| a.invoke_move(1))),
        (500, ui(|a| a.invoke_activate(a.get_focus_index()))),
        (1800, ui(|_| {})),
    ]
}

/// The three layers: a track picker over the playing film, the volume overlay over that, the
/// menu over the stage, and the power hold over everything.
fn layers_script() -> Vec<(u64, Act)> {
    let kodi: Act = Box::new(|_, tx| {
        let _ = tx.send(UiAction::SelectSource("KODI".into()));
    });
    let tap = |x: f32, y: f32| -> Act {
        Box::new(move |a, _| {
            use slint::platform::{PointerEventButton, WindowEvent};
            let position = slint::LogicalPosition::new(x, y);
            let button = PointerEventButton::Left;
            a.window().dispatch_event(WindowEvent::PointerMoved { position });
            a.window().dispatch_event(WindowEvent::PointerPressed { position, button });
            a.window().dispatch_event(WindowEvent::PointerReleased { position, button });
            a.window().dispatch_event(WindowEvent::PointerExited);
        })
    };
    use slint::platform::WindowEvent;
    let down: fn(&AppWindow) = |a| a.window().dispatch_event(WindowEvent::KeyPressed { text: "p".into() });
    let up: fn(&AppWindow) = |a| a.window().dispatch_event(WindowEvent::KeyReleased { text: "p".into() });
    vec![
        (300, kodi),
        (1500, tap(132.0, 746.0)),
        (1600, ui(|a| a.invoke_move(1))),
        (500, key("+")),
        (300, key("+")),
        (2200, press("\n")),
        (1600, tap(348.0, 746.0)),
        (1800, press("\u{8}")),
        (1400, tap(436.0, 32.0)),
        (2600, ui(|a| a.invoke_back())),
        (1500, ui(down)),
        (1500, ui(up)),
        (2400, ui(|_| {})),
    ]
}

/// Main stage to main stage: stop and play on Kodi, change input, and the receiver off and on.
fn stages_script() -> Vec<(u64, Act)> {
    let source = |name: &'static str| -> Act {
        Box::new(move |_, tx| {
            let _ = tx.send(UiAction::SelectSource(name.into()));
        })
    };
    use slint::platform::WindowEvent;
    let down: fn(&AppWindow) = |a| a.window().dispatch_event(WindowEvent::KeyPressed { text: "p".into() });
    let up: fn(&AppWindow) = |a| a.window().dispatch_event(WindowEvent::KeyReleased { text: "p".into() });
    vec![
        (300, source("KODI")),
        (1800, key("s")),
        (1600, key(" ")),
        (1800, source("PLAYSTATION 5")),
        (1600, source("XBOX 360")),
        (1600, source("ANDROID TV")),
        (1600, source("KODI")),
        (1600, ui(down)),
        (1500, ui(up)),
        (2000, ui(down)),
        (1500, ui(up)),
        (2200, ui(|_| {})),
    ]
}

/// Pick an input: the rows lift as focus moves, then the home screen shows the new input at once
/// with a sheen until the receiver confirms.
fn switch_script() -> Vec<(u64, Act)> {
    vec![
        (900, ui(|a| a.go_to(Page::Inputs, a.get_source_index()))),
        (700, ui(|a| a.invoke_move(-1))),
        (600, ui(|a| a.invoke_move(-1))),
        (600, ui(|a| a.invoke_move(1))),
        (600, ui(|a| a.invoke_move(1))),
        (600, ui(|a| a.invoke_move(1))),
        (700, ui(|a| a.invoke_activate(a.get_focus_index()))),
        (3000, ui(|a| a.go_to(Page::Inputs, a.get_source_index()))),
        (700, ui(|a| a.invoke_move(-1))),
        (600, ui(|a| a.invoke_move(-1))),
        (600, ui(|a| a.invoke_move(-1))),
        (700, ui(|a| a.invoke_activate(a.get_focus_index()))),
        (3000, ui(|_| {})),
    ]
}

/// On the charger: the battery screen, then the countdown to dark, then a touch to wake.
fn charge_script() -> Vec<(u64, Act)> {
    vec![
        (1200, key("c")),
        (3500, ui(|_| {})),
        // Nothing touched for ten seconds after it was put down: dark.
        (8500, ui(|_| {})),
        (2000, press("+")),
        (2500, ui(|_| {})),
    ]
}

/// The menu's background, through the input colours.
/// Kodi's library as saved by a fetch script: `library.json` plus a poster file per title, in
/// the directory named by `UC_DEMO_LIBRARY`.
struct Shelf {
    posters: Vec<slint::Image>,
    items: Vec<crate::LibItem>,
    count: String,
}

fn shelf(kind: &str) -> Shelf {
    use serde_json::Value;
    let dir = PathBuf::from(std::env::var("UC_DEMO_LIBRARY").unwrap_or_default());
    let json: Value = std::fs::read(dir.join("library.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let list = json[kind].as_array().cloned().unwrap_or_default();
    let shows = kind == "tvshows";
    let mut posters = Vec::new();
    let mut items = Vec::new();
    for it in &list {
        let Some(file) = it["file"].as_str() else { continue };
        // A missing file stands for a poster still on its way.
        let poster = match image::open(dir.join(file)) {
            Ok(img) => {
                let img = img.resize_to_fill(200, 300, image::imageops::FilterType::Triangle).to_rgb8();
                let poster = slint::Image::from_rgb8(slint::SharedPixelBuffer::clone_from_slice(img.as_raw(), 200, 300));
                posters.push(poster.clone());
                poster
            }
            Err(_) => slint::Image::default(),
        };
        let item: ::kodi::methods::library::FileItem = serde_json::from_value(it.clone()).unwrap_or_default();
        items.push(crate::view::library::item(&drivers::kodi::library_item(&item, shows, String::new()), poster));
    }
    let count = if shows { format!("{} shows", list.len()) } else { format!("{} films", list.len()) };
    Shelf { posters, items, count }
}

fn library_script() -> Vec<(u64, Act)> {
    let movies = Rc::new(shelf("movies"));
    let shows = Rc::new(shelf("tvshows"));
    let show = |shelf: &Rc<Shelf>, heading: &'static str, mode: LibMode| -> Act {
        let shelf = shelf.clone();
        Box::new(move |app, _| {
            app.set_lib_heading(heading.into());
            app.set_lib_count(shelf.count.clone().into());
            app.set_lib_posters(Rc::new(slint::VecModel::from(shelf.posters.clone())).into());
            app.set_lib_items(Rc::new(slint::VecModel::from(shelf.items.clone())).into());
            app.set_lib_focus(0);
            app.show_lib_mode(mode);
        })
    };
    let step = |by: i32| -> Act { Box::new(move |app, _| app.set_lib_focus((app.get_lib_focus() + by).max(0))) };
    let mut s: Vec<(u64, Act)> = vec![(300, show(&movies, "Movies", LibMode::River))];
    s.push((5000, show(&movies, "MOVIES", LibMode::RiverCard)));
    for _ in 0..4 {
        s.push((1500, step(1)));
    }
    s.push((1800, show(&movies, "MOVIES", LibMode::Carousel)));
    for by in [1, 1, 1, -1, 1, 1] {
        s.push((1100, step(by)));
    }
    s.push((1600, show(&movies, "MOVIES", LibMode::Wall)));
    for by in [1, 1, 3, 3, -1, 3, 3, 1] {
        s.push((900, step(by)));
    }
    s.push((1600, show(&shows, "TV SHOWS", LibMode::DimRiverCard)));
    for _ in 0..4 {
        s.push((1500, step(1)));
    }
    s.push((1800, Box::new(|_, _| {})));
    s
}

/// Into the poster wall from the idle home screen and back, as the live wall does it.
fn wall_script() -> Vec<(u64, Act)> {
    let movies = Rc::new(shelf("movies"));
    let load: Act = {
        let shelf = movies.clone();
        Box::new(move |app, tx| {
            let _ = tx.send(UiAction::SelectSource("KODI".into()));
            app.set_lib_heading("MOVIES".into());
            app.set_lib_count(shelf.count.clone().into());
            app.set_lib_items(Rc::new(slint::VecModel::from(shelf.items.clone())).into());
            app.set_lib_focus(4);
        })
    };
    let wall = |on: bool| -> Act { Box::new(move |app, _| crate::view::stage::library_stage(app, if on { LibMode::Wall } else { LibMode::Off })) };
    let step = |by: i32| -> Act { Box::new(move |app, _| app.set_lib_focus((app.get_lib_focus() + by).max(0))) };
    vec![(300, load), (600, key("s")), (1500, wall(true)), (2200, step(1)), (500, step(3)), (900, wall(false)), (1600, wall(true)), (2200, ui(|a| crate::view::stage::reveal_library(a, Duration::ZERO))), (2200, ui(|_| {}))]
}

/// The poster wall settled on one film: the first whose title contains `UC_DEMO_FOCUS`.
fn feature_script() -> Vec<(u64, Act)> {
    let movies = Rc::new(shelf("movies"));
    let want = std::env::var("UC_DEMO_FOCUS").unwrap_or_default().to_lowercase();
    let focus = movies.items.iter().position(|i| !want.is_empty() && i.title.to_lowercase().contains(&want)).unwrap_or(0) as i32;
    let load: Act = Box::new(move |app, tx| {
        let _ = tx.send(UiAction::SelectSource("KODI".into()));
        app.set_lib_heading("MOVIES".into());
        app.set_lib_count(movies.count.clone().into());
        app.set_lib_items(Rc::new(slint::VecModel::from(movies.items.clone())).into());
        app.set_lib_focus(focus);
    });
    vec![(300, load), (600, key("s")), (1500, ui(|a| crate::view::stage::library_stage(a, LibMode::Wall))), (4000, ui(|_| {}))]
}

/// The wall's three ways of showing the highlighted title's details, one after another.
fn walls_script() -> Vec<(u64, Act)> {
    let movies = Rc::new(shelf("movies"));
    let load: Act = {
        let shelf = movies.clone();
        Box::new(move |app, tx| {
            let _ = tx.send(UiAction::SelectSource("KODI".into()));
            app.set_lib_heading("MOVIES".into());
            app.set_lib_count(shelf.count.clone().into());
            app.set_lib_items(Rc::new(slint::VecModel::from(shelf.items.clone())).into());
        })
    };
    let style = |n: i32| -> Act {
        Box::new(move |app, _| {
            crate::view::stage::library_stage(app, LibMode::Off);
            app.set_lib_wall_style(n);
            app.set_lib_focus(12);
        })
    };
    let wall: fn() -> Act = || Box::new(|app, _| crate::view::stage::library_stage(app, LibMode::Wall));
    let step = |by: i32| -> Act { Box::new(move |app, _| app.set_lib_focus((app.get_lib_focus() + by).max(0))) };
    let mut s: Vec<(u64, Act)> = vec![(300, load), (600, key("s"))];
    for n in 1..=3 {
        s.push((1200, style(n)));
        s.push((900, wall()));
        s.push((2000, step(1)));
        s.push((1300, step(3)));
        s.push((1300, step(1)));
        s.push((1300, step(-3)));
    }
    s.push((1800, ui(|_| {})));
    s
}

/// The subtitle picker with a long list: walk down past the screen and back.
fn picker_script() -> Vec<(u64, Act)> {
    let open: Act = Box::new(|app, _| {
        app.show_picker(Picker::Subtitles);
        app.go_to(Page::Tracks, 0);
        let (rows, current) = crate::app::demo::demo_tracks(&crate::app::model::Model::default(), Picker::Subtitles, 1);
        crate::view::show_tracks(&app.as_weak(), rows, current);
    });
    let mut s: Vec<(u64, Act)> = vec![(600, open)];
    for _ in 0..16 {
        s.push((260, ui(|a| a.invoke_move(1))));
    }
    s.push((900, ui(|_| {})));
    for _ in 0..8 {
        s.push((260, ui(|a| a.invoke_move(-1))));
    }
    s.push((1200, ui(|_| {})));
    s
}

fn tap_at(x: f32, y: f32) -> Act {
    Box::new(move |a, _| {
        use slint::platform::{PointerEventButton, WindowEvent};
        let position = slint::LogicalPosition::new(x, y);
        let button = PointerEventButton::Left;
        a.window().dispatch_event(WindowEvent::PointerMoved { position });
        a.window().dispatch_event(WindowEvent::PointerPressed { position, button });
        a.window().dispatch_event(WindowEvent::PointerReleased { position, button });
        a.window().dispatch_event(WindowEvent::PointerExited);
    })
}

/// Short clips for the website's hero, 6 to 9 seconds each after a settling lead-in (`SITE_LEAD`
/// ms, trimmed when the clip is encoded). Each starts from a known input, `UC_SITE_SOURCE` (Kodi by
/// default), so the same clip can be recorded in each input's colours. `site-switch` then moves
/// `UC_SITE_STEPS` rows down the input list and picks that input.
pub const SITE_LEAD: u64 = 2500;

fn site_script(name: &str) -> Vec<(u64, Act)> {
    use slint::platform::WindowEvent;
    let down: fn(&AppWindow) = |a| a.window().dispatch_event(WindowEvent::KeyPressed { text: "p".into() });
    let up: fn(&AppWindow) = |a| a.window().dispatch_event(WindowEvent::KeyReleased { text: "p".into() });
    let start = std::env::var("UC_SITE_SOURCE").unwrap_or_else(|_| "KODI".into());
    let first: Act = Box::new(move |_, tx| {
        let _ = tx.send(UiAction::SelectSource(start.clone()));
    });
    let mut s: Vec<(u64, Act)> = vec![(300, first), (SITE_LEAD - 300, ui(|_| {}))];
    s.extend(match name {
        "site-inputs" => vec![
            (900, ui(|a| a.go_to(Page::Inputs, a.get_source_index()))),
            (700, ui(|a| a.invoke_move(1))),
            (700, ui(|a| a.invoke_activate(a.get_focus_index()))),
            (2300, ui(|a| a.go_to(Page::Inputs, a.get_source_index()))),
            (700, ui(|a| a.invoke_move(1))),
            (700, ui(|a| a.invoke_activate(a.get_focus_index()))),
            (2400, ui(|_| {})),
        ],
        "site-switch" => {
            let steps: i32 = std::env::var("UC_SITE_STEPS").ok().and_then(|n| n.parse().ok()).unwrap_or(1);
            let mut v: Vec<(u64, Act)> = vec![(1400, ui(|a| a.go_to(Page::Inputs, a.get_source_index())))];
            for _ in 0..steps.abs() {
                v.push((550, Box::new(move |a: &AppWindow, _: &Tx| a.invoke_move(steps.signum()))));
            }
            v.push((700, ui(|a| a.invoke_activate(a.get_focus_index()))));
            v.push((3600, ui(|_| {})));
            v
        }
        // The wall scrolling: down three rows, one across, and back up, each a d-pad press sent to Kodi.
        "site-wall" => {
            const UP: &str = "\u{F700}";
            const DOWN: &str = "\u{F701}";
            const RIGHT: &str = "\u{F703}";
            vec![
                (3000, key(DOWN)),
                (1300, key(DOWN)),
                (1300, key(DOWN)),
                (1300, key(RIGHT)),
                (1300, key(UP)),
                (1300, key(UP)),
                (1300, key(UP)),
                (1600, ui(|_| {})),
            ]
        }
        // The remote drives Kodi itself: two d-pad rights along one row of the library to 2001, OK, OK again on
        // Kodi's resume prompt, a few seconds on Now Playing, then the subtitle picker, taken slowly, to English.
        // Once it is on, Kodi is paused and its read-ahead reloaded so the subtitles show (see
        // `kodi::reload`); the pause, marked in the output, is cut from the video.
        "site-kodi-remote" => {
            const RIGHT: &str = "\u{F703}";
            vec![
                (2500, key(RIGHT)),
                (1300, key(RIGHT)),
                (1400, key("\n")),
                (1500, key("\n")),
                (4600, tap_at(132.0, 746.0)),
                (1800, ui(|a| a.invoke_move(1))),
                (1700, ui(|a| a.invoke_activate(a.get_focus_index()))),
                (300, kodi_hold(true)),
                (3000, kodi_hold(false)),
                // Kodi replays the seconds it went back over, which the edit cuts too.
                (6500, ui(|_| {})),
            ]
        }
        // Kodi's library on the wall while a helper steps Kodi across to a film and plays it (about 11 s), then the
        // audio and subtitle pickers.
        "site-kodi" => vec![
            (11000, ui(|_| {})),
            (900, tap_at(348.0, 746.0)),
            (1500, ui(|a| a.invoke_move(1))),
            (900, press("\n")),
            (1400, tap_at(132.0, 746.0)),
            (1400, ui(|a| a.invoke_move(1))),
            (900, ui(|a| a.invoke_activate(a.get_focus_index()))),
            (1600, ui(|_| {})),
        ],
        // The audio picker, then the subtitle picker, held long enough to read their track names.
        "site-tracks" => vec![
            (900, tap_at(348.0, 746.0)),
            (1600, ui(|a| a.invoke_move(1))),
            (900, press("\n")),
            (1500, tap_at(132.0, 746.0)),
            (1500, ui(|a| a.invoke_move(1))),
            (1000, ui(|a| a.invoke_activate(a.get_focus_index()))),
            (1800, ui(|_| {})),
        ],
        "site-playing" => vec![
            (1200, tap_at(240.0, 584.0)),
            (1300, tap_at(240.0, 584.0)),
            (1300, tap_at(348.0, 746.0)),
            (1100, ui(|a| a.invoke_move(1))),
            (600, press("\n")),
            (1300, tap_at(132.0, 746.0)),
            (1100, ui(|a| a.invoke_move(1))),
            (600, ui(|a| a.invoke_activate(a.get_focus_index()))),
            (1500, ui(|_| {})),
        ],
        // Keys tick the level up and down, the slider sweeps it, the overlay fades back to the film, then a
        // double tap mutes and unmutes.
        "site-volume" => vec![
            (900, key("+")),
            (260, key("+")),
            (260, key("+")),
            (260, key("+")),
            (700, key("-")),
            (260, key("-")),
            (900, swipe((0..30).map(|i| 0.35 + i as f32 * 0.012).collect())),
            (2600, tap_strip()),
            (180, tap_strip()),
            (1500, tap_strip()),
            (180, tap_strip()),
            (2000, ui(|_| {})),
        ],
        "site-power" => vec![
            (1000, ui(down)),
            (1500, ui(up)),
            (2200, ui(down)),
            (1500, ui(up)),
            (2200, ui(|_| {})),
        ],
        "site-menu" => vec![
            (900, ui(|a| a.go_to(Page::Menu, 0))),
            (700, ui(|a| a.invoke_move(1))),
            (600, ui(|a| a.invoke_activate(1))),
            (800, ui(|a| a.set_brightness(70))),
            (500, ui(|a| a.set_brightness(45))),
            (600, ui(|a| a.set_brightness(90))),
            (900, ui(|a| a.invoke_back())),
            (600, ui(|a| a.invoke_move(1))),
            (350, ui(|a| a.invoke_move(1))),
            (600, ui(|a| a.invoke_activate(3))),
            (1300, ui(|a| a.invoke_back())),
            (800, ui(|a| a.invoke_back())),
            (1200, ui(|_| {})),
        ],
        "site-library" => {
            let movies = Rc::new(shelf("movies"));
            let want = std::env::var("UC_DEMO_FOCUS").unwrap_or_default().to_lowercase();
            let focus = movies.items.iter().position(|i| !want.is_empty() && i.title.to_lowercase().contains(&want)).unwrap_or(0) as i32;
            let load: Act = Box::new(move |app, _| {
                app.set_lib_heading("MOVIES".into());
                app.set_lib_count(movies.count.clone().into());
                app.set_lib_items(Rc::new(slint::VecModel::from(movies.items.clone())).into());
                app.set_lib_focus((focus - 3).max(0));
            });
            let step = |by: i32| -> Act { Box::new(move |app, _| app.set_lib_focus((app.get_lib_focus() + by).max(0))) };
            vec![
                (100, load),
                (600, key("s")),
                (1500, ui(|a| crate::view::stage::library_stage(a, LibMode::Wall))),
                (1600, step(1)),
                (700, step(1)),
                (700, step(1)),
                (2600, ui(|_| {})),
            ]
        }
        _ => return Vec::new(),
    });
    s
}

fn lava_script() -> Vec<(u64, Act)> {
    let source = |name: &'static str| -> Act {
        Box::new(move |_, tx| {
            let _ = tx.send(UiAction::SelectSource(name.into()));
        })
    };
    vec![
        (200, source("KODI")),
        (4500, source("PLAYSTATION 5")),
        (4000, source("XBOX 360")),
        (4000, source("ANDROID TV")),
        (4000, ui(|_| {})),
    ]
}

/// `--script steps:<file>`: a `--steps` script, filmed. Its pictures aren't taken; a failure to read it ends the
/// recording at once.
fn steps_script(path: &str) -> Vec<(u64, Act)> {
    let timeline = super::steps::load(std::path::Path::new(path)).unwrap_or_else(|e| {
        tracing::error!("{path}: {e:#}");
        Vec::new()
    });
    // Like `--steps`, two seconds for the window and the drivers first.
    std::iter::once((2000, ui(|_| {})))
        .chain(timeline.into_iter().map(|(wait, action)| (wait.as_millis() as u64, Box::new(move |app: &AppWindow, _: &Tx| {
            super::steps::perform(app, &action, None);
        }) as Act)))
        .collect()
}

/// (pause before the step in ms, what happens)
fn script(name: Option<&str>) -> Vec<(u64, Act)> {
    if let Some(path) = name.and_then(|n| n.strip_prefix("steps:")) {
        return steps_script(path);
    }
    match name {
        Some("media") => return media_script(),
        Some("charge") => return charge_script(),
        Some("lava") => return lava_script(),
        Some("layers") => return layers_script(),
        Some("stages") => return stages_script(),
        Some("switch") => return switch_script(),
        Some("library") => return library_script(),
        Some("picker") => return picker_script(),
        Some("wall") => return wall_script(),
        Some("feature") => return feature_script(),
        Some("walls") => return walls_script(),
        Some(n) if n.starts_with("site-") => return site_script(n),
        _ => {}
    }
    if name == Some("home") {
        return home_script();
    }
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
        (1300, ui(|a| a.go_to(Page::Inputs, a.get_source_index()))),
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
        (2000, ui(|a| a.go_to(Page::Menu, 0))),
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
        // Switching input changes the interface colour, and the keys follow.
        (1400, ui(|a| a.invoke_select_source("PLAYSTATION 5".into()))),
        (1700, ui(|a| a.invoke_select_source("XBOX 360".into()))),
        (1700, ui(|a| a.invoke_select_source("ANDROID TV".into()))),
        (1700, ui(|a| a.invoke_select_source("KODI".into()))),
        (1600, ui(|_| {})),
    ]
}

/// Starts the recording; the returned timer must be kept alive until the event loop ends.
pub fn record(app: &AppWindow, tx: Tx, mut device: Device, out: PathBuf, which: Option<&str>) -> anyhow::Result<slint::Timer> {
    let mut ffmpeg = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgba"])
        .args(["-s", &format!("{}x{}", device::WIDTH, device.height())])
        .args(["-framerate", &FPS.to_string(), "-i", "-"])
        // A `.png` pattern such as `frames/%04d.png` keeps every frame lossless, for stills.
        .args(match out.extension().and_then(|e| e.to_str()) {
            Some("png") => &[][..],
            // Lossless with alpha, and quicker to write than PNGs.
            Some("mkv") => &["-c:v", "ffv1", "-level", "3", "-threads", "4", "-slices", "16"][..],
            _ => &["-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "20", "-movflags", "+faststart"][..],
        })
        .arg(&out)
        .stdin(Stdio::piped())
        .spawn()
        .context("starting ffmpeg")?;
    let mut pipe = ffmpeg.stdin.take().context("ffmpeg stdin")?;

    // The UI thread only grabs the screen, often; composing onto the remote and writing happen here. Each
    // output frame shows the latest grab taken at or before its moment, so the video keeps real time evenly
    // whatever a grab costs.
    let (grabs, grabbed) = std::sync::mpsc::sync_channel::<(Instant, Vec<u8>, (u32, u32), backlight::Keys, f32)>(4);
    let writer = std::thread::spawn(move || {
        let mut start = None;
        let mut written = 0u64;
        let mut last: Option<image::RgbaImage> = None;
        let mut write = |frame: &image::RgbaImage| {
            let _ = pipe.write_all(frame.as_raw());
        };
        for (at, pixels, size, keys, dt) in grabbed {
            let frame = device.compose(&pixels, size, keys, dt);
            let start = *start.get_or_insert(at);
            while start + Duration::from_secs_f64(written as f64 / FPS as f64) < at {
                write(last.as_ref().unwrap_or(&frame));
                written += 1;
            }
            last = Some(frame);
        }
        if let Some(frame) = &last {
            write(frame);
        }
        // Closing the pipe is what tells ffmpeg the stream has ended.
        drop(write);
        drop(pipe);
        ffmpeg.wait()
    });

    let mut steps = script(which).into_iter();
    let mut pending = steps.next();
    let mut since = Instant::now();
    let mut last_frame = Instant::now();
    let mut first = true;
    let mut grabs = Some(grabs);
    let mut writer = Some(writer);
    let weak = app.as_weak();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(15), move || {
        let Some(app) = weak.upgrade() else { return };
        if pending.as_ref().is_some_and(|(wait, _)| since.elapsed() >= Duration::from_millis(*wait)) {
            if let Some((_, act)) = pending.take() {
                act(&app, &tx);
                println!("record step {:.3}", unix_now());
            }
            since = Instant::now();
            pending = steps.next();
            if pending.is_none() {
                drop(grabs.take());
                if let Some(writer) = writer.take() {
                    println!("{} {:?}", out.display(), writer.join().map_err(|_| "writer panicked"));
                }
                let _ = slint::quit_event_loop();
                return;
            }
        }
        if let (Some(grabs), Ok(buf)) = (&grabs, app.window().take_snapshot()) {
            let at = Instant::now();
            // Printed once, so a screen recorded elsewhere at the same time can be lined up with this one.
            if first {
                first = false;
                println!("record t0 {:.3}", unix_now());
            }
            let dt = last_frame.elapsed().as_secs_f32();
            last_frame = at;
            let _ = grabs.send((at, buf.as_bytes().to_vec(), (buf.width(), buf.height()), keys(&app), dt));
        }
    });
    Ok(timer)
}

const FPS: u32 = 30;

fn unix_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64()
}
