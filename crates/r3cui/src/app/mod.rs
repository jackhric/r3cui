//! The app's state and its loop: what the UI asks for (`UiAction`), what background work reports back (`Later`),
//! the model the screen shows, and the session that joins the core and the drivers to it.

#[cfg(feature = "demo")]
pub(crate) mod demo;
pub(crate) mod global;
pub(crate) mod integrations;
pub(crate) mod legacy;
pub(crate) mod meter;
pub(crate) mod model;
pub(crate) mod optimistic;
pub(crate) mod registry;
pub(crate) mod session;
pub(crate) mod settings;
pub(crate) mod software;
pub(crate) mod voice;
pub(crate) mod webconfig;

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
    /// Wake the remote as a key press would, for input that does not come through the keys (remote control).
    Wake,
    /// The voice key, down or up, while something takes voice.
    VoiceKey { down: bool },
    VoiceDismiss,
    Software(software::Action),
    /// A browser's request to the web configurator, answered on `reply`.
    Web(webconfig::Call, tokio::sync::oneshot::Sender<webconfig::Answer>),
    /// The web configurator went unused: switch it off.
    WebOff,
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
    Targets { driver: String, kind: r3cui_sdk::api::TargetKind, targets: Vec<r3cui_sdk::api::Target> },
    /// The device answered a pairing's start: it shows its code, or it did not.
    PairStarted(Result<Box<dyn r3cui_sdk::api::Pairer>, r3cui_sdk::api::PairError>),
    /// The pairing's verdict on a code, with the pairing handed back for another try.
    PairChecked(Result<r3cui_sdk::api::Paired, r3cui_sdk::api::PairError>, Box<dyn r3cui_sdk::api::Pairer>),
    /// What the System pages asked the remote or the net for.
    Software(software::Event),
    /// The relay did not end a session's audio in time after the key was let go.
    VoiceEndDue(u32),
    /// The voice key has been held long enough for the core to record.
    VoiceStartDue(u32),
}
