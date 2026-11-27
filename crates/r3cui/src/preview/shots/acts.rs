//! The building blocks of a recorded script: what a step does to the window or sends the session.

use crate::input::Button;
use crate::view::screen::Screen;
use crate::{AppWindow, Lib, PageId, Picker, Tracks, UiAction};
use demo::data::{AUDIO, SUBTITLES, SUBTITLE_SHOWING};
use slint::{ComponentHandle, SharedString};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

pub(super) type Tx = UnboundedSender<UiAction>;
pub(super) type Act = Box<dyn Fn(&AppWindow, &Tx)>;

/// A key from the desktop keyboard: a remote button, or a demo toggle (see `UiAction::DemoKey`).
pub(super) fn key(k: &'static str) -> Act {
    Box::new(move |_, tx| {
        if let Some(action) = crate::input::key_action(k) {
            let _ = tx.send(action);
        }
    })
}

pub(super) fn button(button: Button) -> Act {
    Box::new(move |_, tx| {
        let _ = tx.send(UiAction::Button(button));
    })
}

/// Pause or play on, with the remote's own play/pause key, printing when, for the edit. Pausing also seeks back 3 s,
/// which has Kodi read the subtitles again from the start of the line on show.
pub(super) fn kodi_hold(hold: bool) -> Act {
    Box::new(move |_, tx| {
        println!("record {} {:.3}", if hold { "hold" } else { "release" }, unix_now());
        let _ = tx.send(UiAction::Button(Button::Play));
        if hold {
            let tx = tx.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(600));
                let _ = tx.send(UiAction::Seek(-3));
            });
        }
    })
}

pub(super) fn select_input(name: &'static str) -> Act {
    Box::new(move |_, tx| {
        let _ = tx.send(UiAction::SelectInput(name.into()));
    })
}

/// The physical power key, through real key events.
pub(super) fn power_key(down: bool) -> Act {
    use slint::platform::WindowEvent;
    let text = SharedString::from(crate::input::keys::POWER);
    Box::new(move |a, _| {
        let text = text.clone();
        a.window().dispatch_event(if down { WindowEvent::KeyPressed { text } } else { WindowEvent::KeyReleased { text } });
    })
}

/// The track picker over the playing film, with the demo film's streams.
pub(super) fn open_picker(app: &AppWindow, kind: Picker) {
    app.global::<Tracks>().set_picker(kind);
    app.go_to(PageId::Tracks, 0);
    let (rows, current) = match kind {
        Picker::Audio => (AUDIO.iter().map(|(l, d)| (l.to_string(), d.to_string())).collect(), 0),
        Picker::Subtitles => (
            std::iter::once("Off").chain(SUBTITLES).map(|l| (l.to_string(), String::new())).collect(),
            SUBTITLE_SHOWING + 1,
        ),
    };
    crate::view::show_tracks(&app.as_weak(), rows, current);
}

/// Move the library's highlight by `by`, not above the first title.
pub(super) fn focus_step(by: i32) -> Act {
    Box::new(move |app, _| app.global::<Lib>().set_focus((app.global::<Lib>().get_focus() + by).max(0)))
}

pub(super) fn ui(f: fn(&AppWindow)) -> Act {
    Box::new(move |app, _| f(app))
}

/// A finger on the strip below the screen, run through the real gesture logic: `path` is
/// positions 0..1 sampled every 30 ms, then the finger lifts.
pub(super) fn swipe(path: Vec<f32>) -> Act {
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

pub(super) fn tap_strip() -> Act {
    swipe(vec![0.5, 0.5])
}

pub(super) fn press(text: SharedString) -> Act {
    Box::new(move |a, _| {
        use slint::platform::WindowEvent;
        a.window().dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        a.window().dispatch_event(WindowEvent::KeyReleased { text: text.clone() });
    })
}

pub(super) fn tap_at(x: f32, y: f32) -> Act {
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

pub(super) fn unix_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64()
}
