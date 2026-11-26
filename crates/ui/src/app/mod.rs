//! The app's state and its loop: what the UI asks for (`UiAction`), what background work reports back (`Later`),
//! the model the screen shows, and the session that joins the core and the drivers to it.

pub(crate) mod demo;
pub(crate) mod global;
pub(crate) mod integrations;
pub(crate) mod model;
pub(crate) mod optimistic;
pub(crate) mod session;
pub(crate) mod settings;

use crate::hardware::backlight;
use crate::input::{slider, Button};
use crate::{Picker, PowerAction};
use crate::remote::wifi;

pub(crate) enum UiAction {
    /// A remote button (on the desktop, a key standing in for one).
    Button(Button),
    /// The play button on Now Playing: for the input's UI slot.
    PlayPause,
    /// Desktop only: a key that toggles part of the demo (c charging, x Home Assistant down, z awake).
    DemoKey(char),
    SelectInput(String),
    Power(PowerAction),
    PowerHeld,
    WifiScan,
    WifiBand,
    WifiJoin { ssid: String, hex: String, password: String },
    Backlight(backlight::Keys),
    Seek(i32),
    OpenPicker(Picker),
    /// A row of the picker, by position.
    PickTrack(Picker, usize),
    Slider(slider::Gesture),
    /// Settings → Integrations.
    Integrations(Settings),
    Display { brightness: i32, auto: bool },
}
#[derive(Debug)]
pub(crate) enum Settings {
    Open,
    Pick(usize),
    Back,
    /// The keyboard's Done.
    Text(String),
}

/// Results of work that finishes later than the request that started it.
pub(crate) enum Later {
    Wifi(String, Vec<wifi::Network>),
    JoinDone(bool),
    /// Demo only: the device has caught up with an input change.
    Switched,
    /// The slider's throttled volume command is due.
    SendVolume,
    /// What an Integrations picker offers, as its driver answered.
    Targets(Vec<drivers::api::Target>),
}
