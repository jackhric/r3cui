//! Physical-button model. On the device these come from evdev; on the desktop the keyboard stands in
//! for them. Both feed the same `Button` → command routing.
#[cfg(feature = "device")]
pub(crate) mod evdev;
pub(crate) mod slider;
use crate::{AppWindow, Icons, KeyCell};
use crate::app::UiAction;
use drivers::api::RemoteKey;
use crate::view::keyboard;
use crate::view::screen::{JoinState, Screen};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use tokio::sync::mpsc;

use slint::platform::Key;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Back,
    Home,
    Menu,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
    DpadMiddle,
    VolumeUp,
    VolumeDown,
    Mute,
    ChannelUp,
    ChannelDown,
    Prev,
    Play,
    Next,
    Stop,
    Record,
}

/// Where a button goes: the global volume, or the current input's remote slot. Home, power and the input key are
/// r3cui's own and handled in the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Volume(VolumeKey),
    Remote(RemoteKey),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeKey {
    Up,
    Down,
    Mute,
}

impl Button {
    /// Desktop stand-in for the hard keys: arrows/Enter = d-pad, Backspace/Esc = back, H = home,
    /// M = menu, +/- = volume, 0 = mute, PgUp/PgDn = channel, Space = play, `,` `.` = prev/next,
    /// S = stop, R = record. The power key (P) is handled in the UI, which times the hold.
    pub fn from_key(text: &str) -> Option<Self> {
        let c = text.chars().next()?;
        let is = |k: Key| c == char::from(k);
        Some(match c.to_ascii_lowercase() {
            _ if is(Key::UpArrow) => Self::DpadUp,
            _ if is(Key::DownArrow) => Self::DpadDown,
            _ if is(Key::LeftArrow) => Self::DpadLeft,
            _ if is(Key::RightArrow) => Self::DpadRight,
            _ if is(Key::Return) => Self::DpadMiddle,
            _ if is(Key::Backspace) || is(Key::Escape) => Self::Back,
            _ if is(Key::PageUp) => Self::ChannelUp,
            _ if is(Key::PageDown) => Self::ChannelDown,
            'h' => Self::Home,
            'm' => Self::Menu,
            '+' | '=' => Self::VolumeUp,
            '-' => Self::VolumeDown,
            '0' => Self::Mute,
            ' ' => Self::Play,
            ',' => Self::Prev,
            '.' => Self::Next,
            's' => Self::Stop,
            'r' => Self::Record,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Back => "BACK",
            Self::Home => "HOME",
            Self::Menu => "MENU",
            Self::DpadUp => "DPAD_UP",
            Self::DpadDown => "DPAD_DOWN",
            Self::DpadLeft => "DPAD_LEFT",
            Self::DpadRight => "DPAD_RIGHT",
            Self::DpadMiddle => "DPAD_MIDDLE",
            Self::VolumeUp => "VOLUME_UP",
            Self::VolumeDown => "VOLUME_DOWN",
            Self::Mute => "MUTE",
            Self::ChannelUp => "CHANNEL_UP",
            Self::ChannelDown => "CHANNEL_DOWN",
            Self::Prev => "PREV",
            Self::Play => "PLAY",
            Self::Next => "NEXT",
            Self::Stop => "STOP",
            Self::Record => "RECORD",
        }
    }

    pub fn route(self) -> Option<Route> {
        use {RemoteKey as K, Route::*};
        Some(match self {
            Self::VolumeUp => Volume(VolumeKey::Up),
            Self::VolumeDown => Volume(VolumeKey::Down),
            Self::Mute => Volume(VolumeKey::Mute),
            Self::Home => return None,
            Self::Back => Remote(K::Back),
            Self::Menu => Remote(K::Menu),
            Self::DpadUp => Remote(K::Up),
            Self::DpadDown => Remote(K::Down),
            Self::DpadLeft => Remote(K::Left),
            Self::DpadRight => Remote(K::Right),
            Self::DpadMiddle => Remote(K::Select),
            Self::ChannelUp => Remote(K::ChannelUp),
            Self::ChannelDown => Remote(K::ChannelDown),
            Self::Prev => Remote(K::Previous),
            Self::Play => Remote(K::PlayPause),
            Self::Next => Remote(K::Next),
            Self::Stop => Remote(K::Stop),
            Self::Record => Remote(K::Record),
        })
    }
}

/// A key from the desktop keyboard (or a script): a remote button when it stands for one, otherwise a demo toggle.
pub(crate) fn key_action(text: &str) -> Option<UiAction> {
    match Button::from_key(text) {
        Some(b) => Some(UiAction::Button(b)),
        None => text.chars().next().map(UiAction::DemoKey),
    }
}

/// The keyboard lives on the UI thread: every d-pad press must redraw at once, with no round trip.
pub(crate) fn wire_keyboard(app: &AppWindow, tx: mpsc::UnboundedSender<UiAction>) {
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
                keyboard::Outcome::Done(text) if app.get_kb_purpose() == 1 => {
                    let _ = tx.send(UiAction::Integrations(crate::app::Settings::Text(text)));
                }
                keyboard::Outcome::Done(password) => {
                    let target = app.get_wifi_target();
                    app.show_join(JoinState::Joining);
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
    app.on_kb_start({
        let (kb, after) = (kb.clone(), after.clone());
        move |text| {
            kb.borrow_mut().start(0, "Done", &text);
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
