//! Haptic cues. The firmware gives the UI a path to the motor in `UC_HAPTIC_DEV_PATH`, but what
//! the motor accepts (duration, strength, pattern) is not published, so until that is probed on
//! the device this only records what would be played.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    /// A rumble that builds for as long as the power key is held.
    RumbleStart,
    RumbleStop,
    /// Two quick ticks: the hold completed.
    DoubleTick,
}

impl Cue {
    pub fn from_ui(name: &str) -> Option<Self> {
        match name {
            "hold-start" => Some(Self::RumbleStart),
            "hold-cancel" => Some(Self::RumbleStop),
            "complete" => Some(Self::DoubleTick),
            _ => None,
        }
    }
}

pub fn play(cue: Cue) {
    tracing::info!(?cue, device = ?std::env::var_os("UC_HAPTIC_DEV_PATH"), "haptic");
}
