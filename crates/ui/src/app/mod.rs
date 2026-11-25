//! The app's state and its loop: what the UI asks for (`UiAction`), what background work reports back (`Later`),
//! the model the screen shows, and the session that joins the core and Kodi to it.

pub(crate) mod demo;
pub(crate) mod entity;
pub(crate) mod model;
pub(crate) mod session;
pub(crate) mod settings;
pub(crate) mod title;

use crate::hardware::backlight;
use crate::input::slider;
use crate::integrations::net;

use serde_json::Value;
use std::collections::HashMap;
use std::time::Instant;

pub(crate) enum UiAction {
    Command(String),
    Key(String),
    SelectSource(String),
    Power(String),
    PowerHeld,
    WifiScan,
    WifiBand,
    WifiJoin { ssid: String, hex: String, password: String },
    Backlight(backlight::Keys),
    Seek(i32),
    /// 0 audio, 1 subtitles
    OpenPicker(i32),
    PickTrack(i32, i32),
    Slider(slider::Gesture),
    Display { brightness: i32, auto: bool },
}
/// Results of work that finishes later than the request that started it.
pub(crate) enum Later {
    Wifi(String, Vec<net::Network>),
    Integrations(HashMap<String, bool>),
    /// The core's current list of configured entities.
    Entities(Vec<Value>),
    JoinDone(bool),
    /// Kodi's subtitle streams, as (stream index or None for off, label), and which is on.
    Subtitles(Vec<(Option<i64>, String)>, usize),
    /// Kodi's own title for the media playing, asked whenever the driver's changes.
    Title(String, String),
    /// Kodi's exact position, and when it was read.
    Position(f64, Instant),
    /// Demo only: the receiver has caught up with an input change.
    Switched,
    /// The slider's throttled volume command is due.
    SendVolume,
}
