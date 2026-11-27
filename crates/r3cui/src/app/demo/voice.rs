//! The demo has no microphone: while the voice key is held, a made-up voice feeds the aurora instead, a few
//! syllables a second with breaths between words.

use crate::app::meter::{self, Heard};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

const TICK: Duration = Duration::from_millis(60);
/// Syllables per second.
const RATE: f32 = 4.2;

/// The session talking; 0 for none.
static TALKING: AtomicU32 = AtomicU32::new(0);

pub(crate) fn talk(session: u32) {
    TALKING.store(session, Ordering::SeqCst);
    tokio::spawn(async move {
        let mut t = 0.0f32;
        while TALKING.load(Ordering::SeqCst) == session {
            let syllable = (t * RATE * std::f32::consts::TAU).sin().max(0.0).powf(0.6);
            // A word is about four syllables, then a short breath.
            let word = if (t * RATE / 4.0).fract() < 0.82 { 1.0 } else { 0.15 };
            let level = (0.25 + 0.65 * syllable) * word * (0.85 + 0.15 * (t * 1.3).sin());
            let tilt = 0.5 + 0.5 * (t * 2.1).sin();
            let bands = [0.9 - 0.4 * tilt, 0.7, 0.35 + 0.4 * tilt, 0.25 * syllable].map(|b| (b * level).clamp(0.0, 1.0));
            meter::hear(Heard { level, bands });
            tokio::time::sleep(TICK).await;
            t += TICK.as_secs_f32();
        }
    });
}

pub(crate) fn stop(session: u32) {
    let _ = TALKING.compare_exchange(session, 0, Ordering::SeqCst, Ordering::SeqCst);
}
