//! Physical-button model. On the device these come from evdev; on the desktop the keyboard stands in
//! for them. Both feed the same `Button` → command routing.

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// The entity the screen is showing (Kodi).
    Screen,
    /// The entity the hard volume keys drive (a Home Assistant media_player).
    Volume,
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

    pub fn route(self) -> (Target, &'static str) {
        match self {
            Self::VolumeUp => (Target::Volume, "volume_up"),
            Self::VolumeDown => (Target::Volume, "volume_down"),
            Self::Mute => (Target::Volume, "mute_toggle"),
            Self::Back => (Target::Screen, "back"),
            Self::Home => (Target::Screen, "home"),
            Self::Menu => (Target::Screen, "menu"),
            Self::DpadUp => (Target::Screen, "cursor_up"),
            Self::DpadDown => (Target::Screen, "cursor_down"),
            Self::DpadLeft => (Target::Screen, "cursor_left"),
            Self::DpadRight => (Target::Screen, "cursor_right"),
            Self::DpadMiddle => (Target::Screen, "cursor_enter"),
            Self::ChannelUp => (Target::Screen, "channel_up"),
            Self::ChannelDown => (Target::Screen, "channel_down"),
            Self::Prev => (Target::Screen, "previous"),
            Self::Play => (Target::Screen, "play_pause"),
            Self::Next => (Target::Screen, "next"),
            Self::Stop => (Target::Screen, "stop"),
            Self::Record => (Target::Screen, "record"),
        }
    }
}
