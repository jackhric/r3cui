//! The integrations this build of r3cui carries: the one list of them. An integration is its own crate on
//! `r3cui-sdk`; carrying another is a dependency in Cargo.toml and a line here.

use crate::host::Registry;
use r3cui_sdk::Integration;

const CARRIED: &[Integration] = &[
    homeassistant::driver::INTEGRATION,
    kodi::driver::INTEGRATION,
    androidtv::driver::INTEGRATION,
    #[cfg(feature = "demo")]
    crate::app::demo::integration::INTEGRATION,
];

pub(crate) fn registry() -> Registry {
    Registry::of(CARRIED)
}
