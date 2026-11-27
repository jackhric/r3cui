//! The remote's own outputs: sounds, haptics, key backlight, and the display's frame pacing.

pub(crate) mod audio;
pub(crate) mod backlight;
pub(crate) mod feedback;
#[cfg(feature = "device")]
pub(crate) mod frames;
