//! The app's state and its loop: what the UI asks for (`UiAction`), what background work reports back (`Later`),
//! the model the screen shows, and the session that joins the core and the drivers to it.

#[cfg(feature = "demo")]
pub(crate) mod demo;
pub(crate) mod global;
pub(crate) mod integrations;
pub(crate) mod model;
pub(crate) mod optimistic;
pub(crate) mod session;
pub(crate) mod settings;
pub(crate) mod software;

use crate::hardware::backlight;
use crate::input::{slider, Button};
use crate::{Picker, PowerAction};
use crate::remote::wifi;

pub(crate) enum UiAction {
    /// A remote button (on the desktop, a key standing in for one).
    Button(Button),
    /// The play button on Now Playing: for the input's UI slot.
    PlayPause,
    /// Desktop only: a key that toggles part of the demo (see `demo::toggle`).
    #[cfg_attr(not(feature = "demo"), allow(dead_code))]
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
    Software(software::Action),
}
#[derive(Debug)]
pub(crate) enum Settings {
    Open,
    /// From the empty Inputs page: where its inputs get set up or its source mended.
    OpenForInputs,
    Pick(usize),
    Back,
    /// The keyboard's Done.
    Text(String),
}

/// Results of work that finishes later than the request that started it.
pub(crate) enum Later {
    Wifi(String, Vec<wifi::Network>),
    JoinDone(bool),
    /// The slider's throttled volume command is due.
    SendVolume,
    /// What an Integrations picker offers, as its driver answered.
    Targets { driver: String, kind: drivers::api::TargetKind, targets: Vec<drivers::api::Target> },
    /// The device answered a pairing's start: it shows its code, or it did not.
    PairStarted(Result<Box<dyn drivers::api::Pairer>, drivers::api::PairError>),
    /// The pairing's verdict on a code, with the pairing handed back for another try.
    PairChecked(Result<drivers::api::Paired, drivers::api::PairError>, Box<dyn drivers::api::Pairer>),
    /// What the System pages asked the remote or the net for.
    Software(software::Event),
}
