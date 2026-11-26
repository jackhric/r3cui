//! Everything the screen shows, independent of where it came from.

use drivers::api::{KeyLight, Status};
use crate::remote::{health, wifi};

/// Everything the screen shows, independent of where it came from.
#[derive(Clone, Debug, Default)]
pub(crate) struct Model {
    pub volume: i32,
    /// Something is set up as global volume.
    pub has_volume: bool,
    pub muted: bool,
    /// The input on now, by id.
    pub input: String,
    /// The inputs to choose from, in order, hidden ones left out.
    pub inputs: Vec<InputEntry>,
    /// Why `inputs` is empty.
    pub no_inputs: NoInputs,
    /// The input on now has something in its UI slot: the home page is its stage.
    pub staged: bool,
    /// The input was just picked here and the source has not confirmed it yet.
    pub switching: bool,
    pub title: String,
    pub paused: bool,
    pub duration: f64,
    pub position: f64,
    pub media_format: String,
    pub media_sub: String,
    /// The app in front on the input's screen slot, when it has one.
    pub app: String,
    pub battery: i32,
    /// On the dock or a cable.
    pub charging: bool,
    /// The highest volume allowed (the device's own volume limit), on the same scale as `volume`.
    pub volume_max: i32,
    pub awake: bool,
    pub power_on: bool,
    pub wifi_ssid: String,
    pub wifi: Vec<wifi::Network>,
    /// The preferred Wi-Fi band as the core names it: "a" 5 GHz, "b" 2.4 GHz; empty when unknown.
    pub wifi_band: String,
    pub scanning: bool,
    pub status: Vec<health::StatusLine>,
    pub warning: String,
    pub key_light: KeyLight,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct InputEntry {
    pub id: String,
    pub name: String,
    /// The mark of the driver in its slots; without one, a brand mark is found from the name.
    pub icon: Option<&'static str>,
}

/// Why there are no inputs to choose from.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum NoInputs {
    /// There are some.
    #[default]
    No,
    /// Nothing is set up as global control's inputs (`global`), or every input is hidden.
    SetUp { global: bool },
    /// The integration the inputs come from has not listed them: it is down, connecting or not set up.
    Source { name: String, icon: &'static str, address: Option<String>, status: Status },
}
