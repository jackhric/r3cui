//! Haptic cues. The firmware gives the UI a control file for the motor in `UC_HAPTIC_DEV_PATH`;
//! writing an effect number to it as decimal text plays that effect (this is what the stock UI
//! does: click 1, bump 25, buzz 14, error 45). There is no duration or strength to set, so a
//! "rumble while held" is a train of short effects.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const CLICK: u8 = 1;
const BUMP: u8 = 25;
const RUMBLE_EVERY: Duration = Duration::from_millis(90);
const TICK_GAP: Duration = Duration::from_millis(110);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    /// Repeats for as long as the power key is held.
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

/// Bumped by every cue, so a running rumble stops as soon as anything else is played.
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn effect(id: u8) {
    let Some(path) = std::env::var_os("UC_HAPTIC_DEV_PATH") else { return };
    if let Err(e) = std::fs::write(&path, id.to_string()) {
        tracing::debug!("haptic effect {id}: {e}");
    }
}

pub fn play(cue: Cue) {
    let mine = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    tracing::debug!(?cue, "haptic");
    match cue {
        Cue::RumbleStop => {}
        Cue::RumbleStart => {
            std::thread::spawn(move || {
                // The UI stops it on release or completion; the cap is only a safety net.
                for _ in 0..40 {
                    if GENERATION.load(Ordering::SeqCst) != mine {
                        break;
                    }
                    effect(BUMP);
                    std::thread::sleep(RUMBLE_EVERY);
                }
            });
        }
        Cue::DoubleTick => {
            std::thread::spawn(|| {
                effect(CLICK);
                std::thread::sleep(TICK_GAP);
                effect(CLICK);
            });
        }
    }
}
