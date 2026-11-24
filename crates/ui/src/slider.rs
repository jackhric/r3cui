//! The touch strip below the screen as a volume control: sliding steps the volume, a double tap
//! toggles mute. Pure gesture logic; positions are 0..1 along the strip.

use std::time::{Duration, Instant};

/// How far a finger travels for one volume step: the whole strip is 30 steps.
const STEP: f32 = 1.0 / 30.0;
/// Further than this and a touch is a slide, not a tap.
const TAP_TRAVEL: f32 = 0.05;
const TAP_TIME: Duration = Duration::from_millis(300);
const DOUBLE_TAP_GAP: Duration = Duration::from_millis(400);

#[derive(Debug, PartialEq, Eq)]
pub enum Gesture {
    /// Positive is louder.
    Steps(i32),
    DoubleTap,
}

#[derive(Default)]
pub struct Slider {
    touch: Option<Touch>,
    last_tap: Option<Instant>,
}

struct Touch {
    since: Instant,
    start: f32,
    /// Where the last whole step was taken.
    anchor: f32,
    travelled: f32,
}

impl Slider {
    pub fn feed(&mut self, touching: bool, pos: f32, now: Instant) -> Option<Gesture> {
        match (&mut self.touch, touching) {
            (None, true) => {
                self.touch = Some(Touch { since: now, start: pos, anchor: pos, travelled: 0.0 });
                None
            }
            (Some(t), true) => {
                t.travelled = t.travelled.max((pos - t.start).abs());
                let steps = ((pos - t.anchor) / STEP).trunc() as i32;
                if steps == 0 {
                    return None;
                }
                t.anchor += steps as f32 * STEP;
                Some(Gesture::Steps(steps))
            }
            (Some(t), false) => {
                let tap = t.travelled < TAP_TRAVEL && now.duration_since(t.since) < TAP_TIME;
                self.touch = None;
                if !tap {
                    self.last_tap = None;
                    return None;
                }
                if self.last_tap.is_some_and(|at| now.duration_since(at) < DOUBLE_TAP_GAP) {
                    self.last_tap = None;
                    Some(Gesture::DoubleTap)
                } else {
                    self.last_tap = Some(now);
                    None
                }
            }
            (None, false) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(t0: Instant, n: u64) -> Instant {
        t0 + Duration::from_millis(n)
    }

    #[test]
    fn sliding_steps_both_ways() {
        let t0 = Instant::now();
        let mut s = Slider::default();
        assert_eq!(s.feed(true, 0.5, t0), None);
        assert_eq!(s.feed(true, 0.52, ms(t0, 20)), None);
        assert_eq!(s.feed(true, 0.5 + 2.5 * STEP, ms(t0, 40)), Some(Gesture::Steps(2)));
        assert_eq!(s.feed(true, 0.5 + 3.2 * STEP, ms(t0, 60)), Some(Gesture::Steps(1)));
        assert_eq!(s.feed(true, 0.5, ms(t0, 80)), Some(Gesture::Steps(-3)));
        assert_eq!(s.feed(false, 0.5, ms(t0, 100)), None);
    }

    #[test]
    fn two_quick_taps_mute() {
        let t0 = Instant::now();
        let mut s = Slider::default();
        s.feed(true, 0.3, t0);
        assert_eq!(s.feed(false, 0.3, ms(t0, 80)), None);
        s.feed(true, 0.31, ms(t0, 200));
        assert_eq!(s.feed(false, 0.31, ms(t0, 260)), Some(Gesture::DoubleTap));
        // A third tap starts over.
        s.feed(true, 0.3, ms(t0, 330));
        assert_eq!(s.feed(false, 0.3, ms(t0, 380)), None);
    }

    #[test]
    fn a_slide_between_taps_is_not_a_double_tap() {
        let t0 = Instant::now();
        let mut s = Slider::default();
        s.feed(true, 0.3, t0);
        s.feed(false, 0.3, ms(t0, 60));
        s.feed(true, 0.3, ms(t0, 120));
        s.feed(true, 0.6, ms(t0, 160));
        assert_eq!(s.feed(false, 0.6, ms(t0, 200)), None);
        s.feed(true, 0.6, ms(t0, 260));
        assert_eq!(s.feed(false, 0.6, ms(t0, 300)), None);
    }
}
