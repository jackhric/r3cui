//! Physical-button model. On the device these come from evdev; on the desktop the keyboard stands in
//! for them. Both feed the same `Button` → command routing.
#[cfg(feature = "device")]
pub(crate) mod evdev;
pub(crate) mod inject;
pub(crate) mod slider;
use crate::Nav;
use crate::Wifi;
use crate::Kb;
use crate::{AppWindow, JoinState, KeyGlyph, KbPurpose, KeyCell};
use crate::app::UiAction;
use r3cui_sdk::api::RemoteKey;
use crate::view::keyboard;
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

/// The text each hard key arrives as: from evdev on the device, from the keyboard on the desktop, from scripts in
/// the previews. The window gets the same values as `KeyChars`.
pub(crate) mod keys {
    pub const HOME: &str = "h";
    pub const MENU: &str = "m";
    pub const INPUT: &str = "v";
    pub const POWER: &str = "p";
    pub const VOLUME_UP: &str = "+";
    pub const VOLUME_DOWN: &str = "-";
    pub const MUTE: &str = "0";
    pub const PLAY: &str = " ";
    pub const PREV: &str = ",";
    pub const NEXT: &str = ".";
    pub const STOP: &str = "s";
    pub const RECORD: &str = "r";
}

impl Button {
    /// Desktop stand-in for the hard keys: arrows/Enter = d-pad, Backspace/Esc = back, H = home,
    /// M = menu, +/- = volume, 0 = mute, PgUp/PgDn = channel, Space = play, `,` `.` = prev/next,
    /// S = stop, R = record. The power key (P) is handled in the UI, which times the hold.
    pub fn from_key(text: &str) -> Option<Self> {
        let c = text.chars().next()?;
        let is = |k: Key| c == char::from(k);
        let lower = text[..c.len_utf8()].to_ascii_lowercase();
        Some(match lower.as_str() {
            _ if is(Key::UpArrow) => Self::DpadUp,
            _ if is(Key::DownArrow) => Self::DpadDown,
            _ if is(Key::LeftArrow) => Self::DpadLeft,
            _ if is(Key::RightArrow) => Self::DpadRight,
            _ if is(Key::Return) => Self::DpadMiddle,
            _ if is(Key::Backspace) || is(Key::Escape) => Self::Back,
            _ if is(Key::PageUp) => Self::ChannelUp,
            _ if is(Key::PageDown) => Self::ChannelDown,
            keys::HOME => Self::Home,
            keys::MENU => Self::Menu,
            keys::VOLUME_UP | "=" => Self::VolumeUp,
            keys::VOLUME_DOWN => Self::VolumeDown,
            keys::MUTE => Self::Mute,
            keys::PLAY => Self::Play,
            keys::PREV => Self::Prev,
            keys::NEXT => Self::Next,
            keys::STOP => Self::Stop,
            keys::RECORD => Self::Record,
            _ => return None,
        })
    }

    /// The inverse of [`Button::from_key`].
    pub fn key_text(self) -> SharedString {
        let key = |k: Key| SharedString::from(char::from(k).to_string());
        match self {
            Self::DpadUp => key(Key::UpArrow),
            Self::DpadDown => key(Key::DownArrow),
            Self::DpadLeft => key(Key::LeftArrow),
            Self::DpadRight => key(Key::RightArrow),
            Self::DpadMiddle => key(Key::Return),
            Self::Back => key(Key::Backspace),
            Self::ChannelUp => key(Key::PageUp),
            Self::ChannelDown => key(Key::PageDown),
            Self::Home => keys::HOME.into(),
            Self::Menu => keys::MENU.into(),
            Self::VolumeUp => keys::VOLUME_UP.into(),
            Self::VolumeDown => keys::VOLUME_DOWN.into(),
            Self::Mute => keys::MUTE.into(),
            Self::Play => keys::PLAY.into(),
            Self::Prev => keys::PREV.into(),
            Self::Next => keys::NEXT.into(),
            Self::Stop => keys::STOP.into(),
            Self::Record => keys::RECORD.into(),
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
            let cells: Vec<KeyCell> = kb
                .borrow()
                .cells()
                .into_iter()
                .map(|c| KeyCell {
                    label: c.label.into(),
                    glyph: match c.kind {
                        keyboard::Kind::Shift => KeyGlyph::Shift,
                        keyboard::Kind::Delete => KeyGlyph::Delete,
                        _ => KeyGlyph::None,
                    },
                    x: c.x,
                    y: c.y,
                    w: c.w,
                    h: c.h,
                    focused: c.focused,
                    style: c.style,
                })
                .collect();
            app.global::<Kb>().set_keys(ModelRc::new(VecModel::from(cells)));
            app.global::<Kb>().set_text(kb.borrow().text().into());
            match outcome {
                keyboard::Outcome::Stay => {}
                keyboard::Outcome::Leave => app.global::<Nav>().invoke_back(),
                keyboard::Outcome::Done(text) if app.global::<Kb>().get_purpose() != KbPurpose::WifiPassword => {
                    let _ = tx.send(UiAction::Integrations(crate::app::Settings::Text(text)));
                }
                keyboard::Outcome::Done(password) => {
                    let target = app.global::<Wifi>().get_target();
                    app.global::<Wifi>().set_join_state(JoinState::Joining);
                    let _ = tx.send(UiAction::WifiJoin { ssid: target.ssid.into(), hex: target.ssid_hex.into(), password });
                }
            }
        }
    });
    after(keyboard::Outcome::Stay);
    app.global::<Kb>().on_reset({
        let (kb, after) = (kb.clone(), after.clone());
        move || {
            kb.borrow_mut().reset();
            after(keyboard::Outcome::Stay);
        }
    });
    app.global::<Kb>().on_start({
        let (kb, after, weak) = (kb.clone(), after.clone(), app.as_weak());
        move |text| {
            let Some(app) = weak.upgrade() else { return };
            match app.global::<Kb>().get_purpose() {
                KbPurpose::Code => kb.borrow_mut().start(app.global::<Kb>().get_min_len().max(0) as usize, keyboard::PAIR, &text),
                _ => kb.borrow_mut().start(0, keyboard::DONE, &text),
            }
            after(keyboard::Outcome::Stay);
        }
    });
    app.global::<Kb>().on_nav({
        let (kb, after) = (kb.clone(), after.clone());
        move |name| {
            let outcome = kb.borrow_mut().key(name);
            after(outcome);
        }
    });
    app.global::<Kb>().on_tap(move |index| {
        let outcome = kb.borrow_mut().tap(index.max(0) as usize);
        after(outcome);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_text_is_the_inverse_of_from_key() {
        use Button::*;
        for b in [Back, Home, Menu, DpadUp, DpadDown, DpadLeft, DpadRight, DpadMiddle, VolumeUp, VolumeDown, Mute, ChannelUp, ChannelDown, Prev, Play, Next, Stop, Record] {
            assert_eq!(Button::from_key(&b.key_text()), Some(b));
        }
        assert_eq!(Button::from_key("H"), Some(Home));
    }
}
