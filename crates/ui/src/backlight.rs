//! Which keys light up for what is on screen. The Remote 3 backlight is one RGB colour applied to
//! a chosen set of zones (`set_button_cfg` → `static_color: {rgb, zones}`), so the policy is
//! "which zones", with the colour following the interface accent.

use crate::{AppWindow, Theme};
use slint::ComponentHandle;

/// What the window is showing right now, and the interface accent. UI thread only.
pub fn observe(app: &AppWindow) -> (UiState, [u8; 3]) {
    let accent = app.global::<Theme>().get_accent();
    let state = UiState {
        awake: app.get_awake(),
        power_hold: app.get_power_overlay().then(|| app.get_power_turning_off()),
        receiver_on: app.get_receiver_on(),
        page: app.get_page(),
        kodi: app.get_kodi(),
        playing: !app.get_media_title().is_empty(),
        volume_overlay: app.get_volume_overlay(),
        muted: app.get_muted(),
    };
    (state, [accent.red(), accent.green(), accent.blue()])
}

/// Zone numbers as reported by `/cfg/device/button_layout` on the UCR3, where zone n is named `ZONE_n`.
pub const ZONE_COUNT: u8 = 12;

pub mod zone {
    pub const HOME: u8 = 1;
    pub const VOLUME_UP_BACK: u8 = 2;
    pub const DPAD_UP: u8 = 3;
    pub const CHANNEL_UP_POWER: u8 = 4;
    pub const DPAD_LEFT: u8 = 5;
    pub const DPAD_RIGHT: u8 = 6;
    pub const DPAD_DOWN: u8 = 7;
    pub const VOLUME_DOWN: u8 = 8;
    pub const CHANNEL_DOWN: u8 = 9;
    pub const MUTE_PREV: u8 = 10;
    pub const RECORD_MENU_STOP_PLAY: u8 = 11;
    pub const VOICE_NEXT: u8 = 12;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiState {
    pub awake: bool,
    /// While the power key is held: `Some(true)` turning off, `Some(false)` turning on.
    pub power_hold: Option<bool>,
    pub receiver_on: bool,
    /// 0 home, anything else is a list page (inputs, menu).
    pub page: i32,
    pub kodi: bool,
    pub playing: bool,
    pub volume_overlay: bool,
    pub muted: bool,
}

pub fn zones(s: UiState) -> &'static [u8] {
    use zone::*;
    const VOLUME: &[u8] = &[VOLUME_UP_BACK, VOLUME_DOWN, MUTE_PREV];
    const LIST: &[u8] = &[HOME, VOLUME_UP_BACK, DPAD_UP, DPAD_LEFT, DPAD_RIGHT, DPAD_DOWN];
    const PLAYING: &[u8] = &[VOLUME_UP_BACK, VOLUME_DOWN, MUTE_PREV, RECORD_MENU_STOP_PLAY, VOICE_NEXT];
    const BROWSING: &[u8] =
        &[HOME, VOLUME_UP_BACK, DPAD_UP, DPAD_LEFT, DPAD_RIGHT, DPAD_DOWN, VOLUME_DOWN, RECORD_MENU_STOP_PLAY];

    if !s.awake {
        &[]
    } else if s.power_hold.is_some() {
        &[CHANNEL_UP_POWER]
    } else if s.page != 0 {
        LIST
    } else if !s.receiver_on {
        &[CHANNEL_UP_POWER]
    } else if s.muted && (s.volume_overlay || !s.kodi) {
        &[MUTE_PREV]
    } else if s.volume_overlay || !s.kodi {
        VOLUME
    } else if s.playing {
        PLAYING
    } else {
        BROWSING
    }
}

/// The one backlight colour: the interface accent, except while the power key is held, when it
/// matches the overlay (red to turn off, green to turn on).
pub fn colour(s: UiState, accent: [u8; 3]) -> [u8; 3] {
    match s.power_hold {
        Some(true) => [0xE8, 0x34, 0x3A],
        Some(false) => [0x3D, 0xDC, 0x84],
        None => accent,
    }
}
