//! Haptic and audio feedback. The rest of the UI calls `play(Cue::…)`; a worker thread owns the
//! motor and the speaker, so nothing here blocks the caller. Patterns are data (see `Cue::spec`),
//! and `Player` is the pure scheduler behind them, driven with an explicit clock.
//!
//! The firmware gives the UI a control file for the motor in `UC_HAPTIC_DEV_PATH`; writing an
//! effect number to it as decimal text plays that effect. The numbers are the DRV2605 ROM
//! waveform ids (see docs/haptics.md), unverified except 1, 14 and 25.

mod cue;
mod device;
mod lab;
mod player;
mod worker;

use crate::Haptic;
use cue::{cue_for, Cue};

pub(crate) use lab::Lab;
pub(crate) use worker::{play, set_core_config, stop};

pub(crate) const SLIDER_MUTE: Cue = Cue::Select;
/// Every volume change, from the keys or the slider.
pub(crate) const VOLUME: Cue = Cue::Volume;
pub(crate) const SLIDER_LIMIT: Cue = Cue::Limit;
/// The voice key held, as the Home key's hold: a ramp while the microphone powers up, then the cue to talk.
pub(crate) const VOICE_WARMING: Cue = Cue::HoldRamp;
pub(crate) const VOICE_LIVE: Cue = Cue::HoldComplete;

/// Run an event from the screen.
pub(crate) fn ui_event(event: Haptic) {
    match cue_for(event) {
        Some(cue) => play(cue),
        None => worker::stop(),
    }
}
