//! Device input. The remote has no libinput, so the touchscreen and the hard keys are read from
//! evdev here and handed to the window as ordinary pointer and key events. Keys are translated to
//! the same key text the desktop build uses, so everything downstream is shared.

use crate::input::{keys, Button};
use crate::{AppWindow, app::UiAction};
use tokio::sync::mpsc::UnboundedSender;
use evdev::{Device, EventType};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, SharedString};

const SCREEN: (f32, f32) = (480.0, 800.0);
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const BTN_TOUCH: u16 = 0x14a;

/// Linux key code → the key text the UI understands. The firmware's own table is not published;
/// these are the standard codes for the names the stock UI uses, and unknown codes are logged.
fn key_text(code: u16) -> Option<SharedString> {
    let button = |b: Button| Some(b.key_text());
    let text = |s: &str| Some(SharedString::from(s));
    match code {
        103 => button(Button::DpadUp),
        108 => button(Button::DpadDown),
        105 => button(Button::DpadLeft),
        106 => button(Button::DpadRight),
        28 | 96 | 352 | 353 => button(Button::DpadMiddle), // ENTER, KPENTER, OK, SELECT
        158 | 174 | 1 => button(Button::Back),             // BACK, EXIT, ESC
        102 | 172 => text(keys::HOME),                     // HOME, HOMEPAGE
        115 => text(keys::VOLUME_UP),
        114 => text(keys::VOLUME_DOWN),
        113 => text(keys::MUTE),
        402 => button(Button::ChannelUp),                  // CHANNELUP
        403 => button(Button::ChannelDown),
        165 | 168 | 412 => text(keys::PREV),               // PREVIOUSSONG, REWIND, PREVIOUS
        164 | 207 | 200 | 201 => text(keys::PLAY),         // PLAYPAUSE, PLAY, PLAYCD, PAUSECD
        163 | 208 | 407 => text(keys::NEXT),               // NEXTSONG, FASTFORWARD, NEXT
        128 | 166 => text(keys::STOP),                     // STOP, STOPCD
        167 => text(keys::RECORD),                         // RECORD
        139 | 62 | 127 => text(keys::MENU),                // MENU, F4, COMPOSE
        116 => text(keys::POWER),                          // POWER
        61 | 217 | 248 | 582 => text(keys::INPUT),         // F3, SEARCH, MICMUTE, VOICECOMMAND: the voice key
        _ => None,
    }
}

pub fn start(app: &AppWindow, tx: UnboundedSender<UiAction>) {
    let mut found = 0;
    for (path, device) in evdev::enumerate() {
        let name = device.name().unwrap_or("?").to_string();
        // The slider below the screen also has X/Y axes and must not be mistaken for the panel.
        let lower = name.to_lowercase();
        let is_touch = (device.properties().contains(evdev::PropType::DIRECT) || lower.contains("touchscreen"))
            && !lower.contains("slider")
            && device.supported_absolute_axes().is_some_and(|a| a.iter().any(|x| x.0 == ABS_MT_POSITION_X || x.0 == ABS_X));
        if lower.contains("slider") {
            tracing::info!("input {}: {name:?} (touch slider)", path.display());
            let (weak, tx) = (app.as_weak(), tx.clone());
            std::thread::spawn(move || read_slider(device, weak, tx));
            continue;
        }
        let keys = device.supported_keys().map_or(0, |k| k.iter().count());
        tracing::info!("input {}: {name:?}, touch={is_touch}, keys={keys}", path.display());
        if !is_touch && keys == 0 {
            continue;
        }
        found += 1;
        let weak = app.as_weak();
        std::thread::spawn(move || read(device, is_touch, weak));
    }
    if found == 0 {
        tracing::warn!("no input devices could be opened");
    }
}

/// The capacitive strip below the screen: slide for volume, double-tap to mute.
fn read_slider(mut device: Device, weak: slint::Weak<AppWindow>, tx: UnboundedSender<UiAction>) {
    let ranges = device.get_abs_state().ok();
    let max_of = |code: u16| ranges.as_ref().map(|r| r[code as usize].maximum).filter(|m| *m > 0);
    let mut gestures = crate::input::slider::Slider::default();
    let mut values: std::collections::BTreeMap<u16, i32> = Default::default();
    let mut touching = false;
    loop {
        let events = match device.fetch_events() {
            Ok(events) => events.collect::<Vec<_>>(),
            Err(e) => {
                tracing::warn!("slider read failed: {e}");
                return;
            }
        };
        for ev in events {
            let (kind, code, value) = (ev.event_type(), ev.code(), ev.value());
            if kind == EventType::KEY {
                touching = value != 0;
            } else if kind == EventType::ABSOLUTE {
                if code == ABS_MT_TRACKING_ID {
                    touching = value >= 0;
                } else {
                    values.insert(code, value);
                }
            } else if kind == EventType::SYNCHRONIZATION {
                // Whichever position axis it uses, the first one reported is the length of the strip.
                let position = [ABS_X, ABS_MT_POSITION_X, ABS_Y, ABS_MT_POSITION_Y]
                    .into_iter()
                    .find_map(|c| Some(*values.get(&c)? as f32 / max_of(c).unwrap_or(479) as f32))
                    .unwrap_or(0.0)
                    .clamp(0.0, 1.0);
                if touching {
                    activity(&weak);
                }
                if let Some(g) = gestures.feed(touching, position, std::time::Instant::now()) {
                    let _ = tx.send(UiAction::Slider(g));
                }
            }
        }
    }
}

/// Input wakes a screen resting on the charger; the touch or key that does so is still delivered,
/// and the window decides whether it only wakes.
fn activity(weak: &slint::Weak<AppWindow>) {
    crate::view::idle::poke();
    let weak = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(app) = weak.upgrade() {
            crate::hardware::frames::kick();
            if app.get_resting() {
                app.set_resting(false);
            }
        }
    });
}

fn send(weak: &slint::Weak<AppWindow>, event: WindowEvent) {
    crate::view::idle::poke();
    let weak = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(app) = weak.upgrade() {
            crate::hardware::frames::kick();
            app.window().dispatch_event(event);
        }
    });
}

fn read(mut device: Device, is_touch: bool, weak: slint::Weak<AppWindow>) {
    let range = |code: u16| {
        device.get_abs_state().ok().map(|s| s[code as usize].maximum).filter(|m| *m > 0).unwrap_or(0) as f32
    };
    let multitouch = range(ABS_MT_POSITION_X) > 0.0;
    let (max_x, max_y) = if multitouch {
        (range(ABS_MT_POSITION_X), range(ABS_MT_POSITION_Y))
    } else {
        (range(ABS_X), range(ABS_Y))
    };
    if is_touch {
        tracing::info!("touch range {max_x}x{max_y}, multitouch={multitouch}");
    }

    let (mut x, mut y) = (0.0f32, 0.0f32);
    let (mut down, mut was_down, mut moved) = (false, false, false);
    loop {
        let events = match device.fetch_events() {
            Ok(events) => events.collect::<Vec<_>>(),
            Err(e) => {
                tracing::warn!("input read failed: {e}");
                return;
            }
        };
        for ev in events {
            let (kind, code, value) = (ev.event_type(), ev.code(), ev.value());
            if kind == EventType::KEY && code == BTN_TOUCH {
                down = value != 0;
            } else if kind == EventType::KEY {
                // Timing for diagnosing slow presses: compare with the command log lines.
                if value != 2 {
                    tracing::info!("key {code} {}", if value == 1 { "down" } else { "up" });
                }
                match key_text(code) {
                    Some(text) => send(&weak, match value {
                        0 => WindowEvent::KeyReleased { text },
                        1 => WindowEvent::KeyPressed { text },
                        _ => WindowEvent::KeyPressRepeated { text },
                    }),
                    // 143 is the wake signal from the power and motion sensors, not a key.
                    None if value == 1 && code != 143 => tracing::info!("unmapped key code {code}"),
                    None => {}
                }
            } else if kind == EventType::ABSOLUTE && is_touch {
                match code {
                    ABS_MT_POSITION_X | ABS_X if max_x > 0.0 => (x, moved) = (value as f32 / max_x * SCREEN.0, true),
                    ABS_MT_POSITION_Y | ABS_Y if max_y > 0.0 => (y, moved) = (value as f32 / max_y * SCREEN.1, true),
                    ABS_MT_TRACKING_ID => down = value >= 0,
                    _ => {}
                }
            } else if kind == EventType::SYNCHRONIZATION && is_touch {
                let position = LogicalPosition::new(x, y);
                let button = PointerEventButton::Left;
                if down && !was_down {
                    send(&weak, WindowEvent::PointerMoved { position });
                    send(&weak, WindowEvent::PointerPressed { position, button });
                } else if down && moved {
                    send(&weak, WindowEvent::PointerMoved { position });
                } else if !down && was_down {
                    send(&weak, WindowEvent::PointerReleased { position, button });
                    send(&weak, WindowEvent::PointerExited);
                }
                (was_down, moved) = (down, false);
            }
        }
    }
}
