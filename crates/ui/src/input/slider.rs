//! The touch strip below the screen as a volume control: sliding steps the volume, a double tap
//! toggles mute. Pure gesture logic; positions are 0..1 along the strip.

use std::time::{Duration, Instant};

/// How far a finger travels for one step: the whole strip is 100 steps, which the caller maps
/// onto the device's usable range.
const STEP: f32 = 1.0 / 100.0;
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

/// Paces volume commands to what the device can take: one in flight at a time, at least
/// `min_gap` apart. The next goes out once the device reports the last one, or after `max_wait`
/// if it never does. Changes in between are merged, and the latest always goes out, so the
/// device heads for wherever the finger is now and never builds up a queue.
pub struct Pacer {
    min_gap: Duration,
    max_wait: Duration,
    sent: Option<(i32, Instant)>,
    confirmed: bool,
    /// The level the device last reported.
    level: Option<i32>,
    pending: Option<i32>,
    /// Moves folded into the next send, for the log.
    merged: u32,
    /// When the timer already asked for will fire. An extra timer is harmless; one firing too
    /// late is not, so a sooner moment always asks again.
    scheduled: Option<Instant>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Send {
    Now(i32),
    /// Call `due` after this long.
    Later(Duration),
    /// Nothing to do now; a send is already scheduled, or there is nothing left to send.
    Wait,
}

impl Pacer {
    pub fn new(min_gap: Duration, max_wait: Duration) -> Self {
        Self { min_gap, max_wait, sent: None, confirmed: true, level: None, pending: None, merged: 0, scheduled: None }
    }

    pub fn offer(&mut self, value: i32, now: Instant) -> Send {
        self.pending = Some(value);
        self.merged += 1;
        self.next(now)
    }

    /// The device reported this level.
    /// How many changes were merged into the send just made, for the log; counting starts again.
    pub fn take_merged(&mut self) -> u32 {
        std::mem::take(&mut self.merged)
    }

    pub fn reported(&mut self, value: i32, now: Instant) -> Send {
        self.level = Some(value);
        if self.sent.is_some_and(|(v, _)| v == value) {
            self.confirmed = true;
        }
        self.next(now)
    }

    /// The scheduled moment has come.
    pub fn due(&mut self, now: Instant) -> Send {
        if self.scheduled.is_some_and(|at| at <= now) {
            self.scheduled = None;
        }
        self.next(now)
    }

    fn next(&mut self, now: Instant) -> Send {
        let Some(value) = self.pending else { return Send::Wait };
        // The device reports only changes, so asking for the level it is already at would
        // never be answered. Safe to skip only when nothing else is on its way.
        if self.confirmed && self.level == Some(value) {
            self.pending = None;
            return Send::Wait;
        }
        if let Some((_, at)) = self.sent {
            let ready = at + if self.confirmed { self.min_gap } else { self.max_wait };
            if now < ready {
                if self.scheduled.is_some_and(|at| at <= ready) {
                    return Send::Wait;
                }
                self.scheduled = Some(ready);
                return Send::Later(ready - now);
            }
        }
        self.pending = None;
        self.sent = Some((value, now));
        self.confirmed = false;
        Send::Now(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(t0: Instant, n: u64) -> Instant {
        t0 + Duration::from_millis(n)
    }

    #[test]
    fn pacer_waits_for_the_device_and_sends_only_the_latest() {
        let t0 = Instant::now();
        let mut p = Pacer::new(Duration::from_millis(250), Duration::from_millis(1000));
        assert_eq!(p.offer(60, t0), Send::Now(60));
        // Not confirmed yet: wait up to a second, merging what comes in.
        assert_eq!(p.offer(61, ms(t0, 50)), Send::Later(Duration::from_millis(950)));
        assert_eq!(p.offer(64, ms(t0, 100)), Send::Wait);
        // The device reports 60 after 400 ms: the latest goes out at once (past the 250 ms gap).
        assert_eq!(p.reported(60, ms(t0, 400)), Send::Now(64));
        // Confirmed quickly: the next still keeps the minimum gap.
        assert_eq!(p.reported(64, ms(t0, 450)), Send::Wait);
        assert_eq!(p.offer(66, ms(t0, 500)), Send::Later(Duration::from_millis(150)));
        assert_eq!(p.due(ms(t0, 650)), Send::Now(66));
        // A device that never answers costs a second, not a queue.
        assert_eq!(p.offer(70, ms(t0, 700)), Send::Later(Duration::from_millis(950)));
        assert_eq!(p.due(ms(t0, 1650)), Send::Now(70));
        // Stale reports of other levels do not count as confirmation.
        assert_eq!(p.offer(71, ms(t0, 1700)), Send::Later(Duration::from_millis(950)));
        assert_eq!(p.reported(43, ms(t0, 1800)), Send::Wait);
    }

    #[test]
    fn pacer_skips_the_level_the_device_is_at() {
        let t0 = Instant::now();
        let mut p = Pacer::new(Duration::from_millis(250), Duration::from_millis(600));
        p.reported(72, t0);
        assert_eq!(p.offer(72, ms(t0, 10)), Send::Wait);
        assert_eq!(p.offer(70, ms(t0, 20)), Send::Now(70));
        // Back to 72 while 70 is still on its way: 72 must still go, or the device ends at 70.
        assert_eq!(p.offer(72, ms(t0, 30)), Send::Later(Duration::from_millis(590)));
        assert_eq!(p.reported(70, ms(t0, 300)), Send::Now(72));
    }

    #[test]
    fn sliding_steps_both_ways() {
        let t0 = Instant::now();
        let mut s = Slider::default();
        assert_eq!(s.feed(true, 0.5, t0), None);
        assert_eq!(s.feed(true, 0.5 + 0.5 * STEP, ms(t0, 20)), None);
        assert_eq!(s.feed(true, 0.5 + 2.5 * STEP, ms(t0, 40)), Some(Gesture::Steps(2)));
        assert_eq!(s.feed(true, 0.5 + 3.2 * STEP, ms(t0, 60)), Some(Gesture::Steps(1)));
        assert_eq!(s.feed(true, 0.5 - 0.4 * STEP, ms(t0, 80)), Some(Gesture::Steps(-3)));
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
