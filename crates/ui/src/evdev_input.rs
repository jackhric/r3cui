//! Device input. The remote has no libinput, so the touchscreen and the hard keys are read from
//! evdev here and handed to the window as ordinary pointer and key events. Keys are translated to
//! the same key text the desktop build uses, so everything downstream is shared.

use crate::AppWindow;
use evdev::{Device, EventType};
use slint::platform::{Key, PointerEventButton, WindowEvent};
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
    let key = |k: Key| Some(SharedString::from(String::from(char::from(k))));
    let text = |s: &str| Some(SharedString::from(s));
    match code {
        103 => key(Key::UpArrow),
        108 => key(Key::DownArrow),
        105 => key(Key::LeftArrow),
        106 => key(Key::RightArrow),
        28 | 96 | 352 | 353 => key(Key::Return), // ENTER, KPENTER, OK, SELECT
        158 | 174 | 1 => key(Key::Backspace),    // BACK, EXIT, ESC
        102 | 172 => text("h"),                  // HOME, HOMEPAGE
        115 => text("+"),
        114 => text("-"),
        113 => text("0"),                        // MUTE
        402 => key(Key::PageUp),                 // CHANNELUP
        403 => key(Key::PageDown),
        165 | 168 | 412 => text(","),            // PREVIOUSSONG, REWIND, PREVIOUS
        164 | 207 | 200 | 201 => text(" "),      // PLAYPAUSE, PLAY, PLAYCD, PAUSECD
        163 | 208 | 407 => text("."),            // NEXTSONG, FASTFORWARD, NEXT
        128 | 166 => text("s"),                  // STOP, STOPCD
        167 => text("r"),                        // RECORD
        139 | 62 | 127 => text("m"),             // MENU, F4, COMPOSE
        116 => text("p"),                        // POWER
        61 | 217 | 248 | 582 => text("v"),         // F3, SEARCH, MICMUTE, VOICECOMMAND: the voice key
        _ => None,
    }
}

pub fn start(app: &AppWindow) {
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
            let weak = app.as_weak();
            std::thread::spawn(move || read_slider(device, weak));
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

/// The capacitive slider below the screen. For now this only shows what it reports, so the
/// hardware can be explored: every axis it sends, and the position along its length.
fn read_slider(mut device: Device, weak: slint::Weak<AppWindow>) {
    let ranges = device.get_abs_state().ok();
    let max_of = |code: u16| ranges.as_ref().map(|r| r[code as usize].maximum).filter(|m| *m > 0);
    let axes: Vec<String> = device
        .supported_absolute_axes()
        .map(|a| a.iter().map(|x| format!("{:#04x} max {:?}", x.0, max_of(x.0))).collect())
        .unwrap_or_default();
    tracing::info!("slider axes: {}", axes.join(", "));

    let mut values: std::collections::BTreeMap<u16, i32> = Default::default();
    let mut touching = false;
    let mut logged = 0;
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
            if logged < 60 && kind != EventType::SYNCHRONIZATION {
                logged += 1;
                tracing::info!("slider event type {:?} code {code:#05x} value {value}", kind);
            }
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
                    .find_map(|c| Some(*values.get(&c)? as f32 / max_of(c).unwrap_or(300) as f32));
                let raw = values.iter().map(|(c, v)| format!("{c:#04x}={v}")).collect::<Vec<_>>().join("  ");
                let (weak, touch, pos) = (weak.clone(), touching, position.unwrap_or(0.0).clamp(0.0, 1.0));
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(app) = weak.upgrade() {
                        crate::frames::kick();
                        app.set_slider_touch(touch);
                        app.set_slider_pos(pos);
                        app.set_slider_raw(raw.into());
                    }
                });
            }
        }
    }
}

fn send(weak: &slint::Weak<AppWindow>, event: WindowEvent) {
    let weak = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(app) = weak.upgrade() {
            crate::frames::kick();
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
