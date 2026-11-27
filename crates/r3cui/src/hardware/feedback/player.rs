use super::cue::{Cue, Spec, Strength};
use std::time::{Duration, Instant};

const FULL_INTENSITY_FROM: f32 = 0.67;
const MEDIUM_INTENSITY_FROM: f32 = 0.34;
const DEFAULT_MAX_MS: u64 = 60_000;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Start {
    Playing,
    Dropped,
    Off,
}

struct Active {
    cue: Cue,
    spec: Spec,
    strength: Strength,
    next: usize,
    due: Instant,
    ends: Instant,
}

/// The motor's schedule: which effect to write when. No I/O and no clock of its own.
pub(super) struct Player {
    active: Option<Active>,
    last: [Option<Instant>; Cue::ALL.len()],
    pub(super) enabled: bool,
    /// 0..=1; lowers every strength by up to two notches.
    pub(super) intensity: f32,
}

impl Default for Player {
    fn default() -> Self {
        Self { active: None, last: [None; Cue::ALL.len()], enabled: true, intensity: 1.0 }
    }
}

impl Player {
    fn expire(&mut self, now: Instant) {
        if let Some(a) = &self.active {
            let finished = a.next >= a.spec.steps.len() && !a.spec.repeat && now >= a.due;
            if finished || now >= a.ends {
                self.active = None;
            }
        }
    }

    fn scaled(&self, strength: Strength) -> Strength {
        strength.shifted(match self.intensity {
            i if i >= FULL_INTENSITY_FROM => 0,
            i if i >= MEDIUM_INTENSITY_FROM => -1,
            _ => -2,
        })
    }

    pub(super) fn start(&mut self, cue: Cue, strength: Option<Strength>, now: Instant) -> Start {
        if !self.enabled || self.intensity <= 0.0 {
            return Start::Off;
        }
        self.expire(now);
        let spec = cue.spec();
        if let Some(last) = self.last[cue.index()] {
            if now.duration_since(last) < Duration::from_millis(spec.min_gap_ms) {
                return Start::Dropped;
            }
        }
        if let Some(a) = &self.active {
            if spec.priority < a.spec.priority || (spec.drop_if_busy && spec.priority == a.spec.priority) {
                return Start::Dropped;
            }
        }
        self.last[cue.index()] = Some(now);
        self.active = Some(Active {
            cue,
            spec,
            strength: self.scaled(strength.unwrap_or(spec.strength)),
            next: 0,
            due: now,
            ends: now + Duration::from_millis(if spec.max_ms > 0 { spec.max_ms } else { DEFAULT_MAX_MS }),
        });
        Start::Playing
    }

    /// Ends a rumble or ramp; a short one-shot such as a confirm is left to finish.
    pub(super) fn stop(&mut self) -> bool {
        match &self.active {
            Some(a) if a.cue.stoppable() => {
                self.active = None;
                true
            }
            _ => false,
        }
    }

    pub(super) fn cancel(&mut self) {
        self.active = None;
    }

    /// The effect to write now, if one is due.
    pub(super) fn poll(&mut self, now: Instant) -> Option<u8> {
        self.expire(now);
        let a = self.active.as_mut()?;
        if now < a.due {
            return None;
        }
        if a.next >= a.spec.steps.len() {
            a.next = 0;
        }
        let s = a.spec.steps[a.next];
        a.next += 1;
        a.due = now + Duration::from_millis(s.after_ms as u64);
        Some(s.effect.id(a.strength.shifted(s.boost)))
    }

    /// When `poll` next has something to do.
    pub(super) fn next_due(&self) -> Option<Instant> {
        self.active.as_ref().map(|a| a.due.min(a.ends))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn drain(p: &mut Player, from: Instant, to_ms: u64) -> Vec<(u64, u8)> {
        let mut out = vec![];
        for t in 0..=to_ms {
            let now = from + ms(t);
            while let Some(id) = p.poll(now) {
                out.push((t, id));
            }
        }
        out
    }

    #[test]
    fn success_is_two_ticks_apart() {
        let (mut p, t0) = (Player::default(), Instant::now());
        assert_eq!(p.start(Cue::Success, None, t0), Start::Playing);
        assert_eq!(drain(&mut p, t0, 400), vec![(0, 25), (110, 25)]);
    }

    #[test]
    fn strength_picks_the_variant() {
        let (mut p, t0) = (Player::default(), Instant::now());
        p.start(Cue::Tap, Some(Strength::Soft), t0);
        assert_eq!(p.poll(t0), Some(3));
        p.start(Cue::Tap, Some(Strength::Strong), t0 + ms(500));
        assert_eq!(p.poll(t0 + ms(500)), Some(1));
    }

    #[test]
    fn intensity_lowers_strength() {
        let (mut p, t0) = (Player::default(), Instant::now());
        p.intensity = 0.5;
        p.start(Cue::Select, None, t0);
        assert_eq!(p.poll(t0), Some(2), "strong click becomes medium");
        p.intensity = 0.0;
        assert_eq!(p.start(Cue::Select, None, t0 + ms(500)), Start::Off);
    }

    #[test]
    fn disabled_plays_nothing() {
        let (mut p, t0) = (Player::default(), Instant::now());
        p.enabled = false;
        assert_eq!(p.start(Cue::Error, None, t0), Start::Off);
        assert_eq!(p.poll(t0), None);
    }

    #[test]
    fn tick_is_dropped_while_busy_and_rate_limited() {
        let (mut p, t0) = (Player::default(), Instant::now());
        assert_eq!(p.start(Cue::Tick, None, t0), Start::Playing);
        assert!(p.poll(t0).is_some());
        assert_eq!(p.start(Cue::Tick, None, t0 + ms(10)), Start::Dropped);
        assert_eq!(p.start(Cue::Tick, None, t0 + ms(60)), Start::Playing, "the first has finished by then");
        assert_eq!(p.start(Cue::Tick, None, t0 + ms(80)), Start::Dropped, "min gap");
    }

    #[test]
    fn select_interrupts_a_tick_but_not_the_reverse() {
        let (mut p, t0) = (Player::default(), Instant::now());
        p.start(Cue::Select, None, t0);
        assert_eq!(p.start(Cue::Tick, None, t0 + ms(5)), Start::Dropped);
        let mut q = Player::default();
        q.start(Cue::Tick, None, t0);
        assert_eq!(q.start(Cue::Select, None, t0 + ms(5)), Start::Playing);
        assert_eq!(q.poll(t0 + ms(5)), Some(1), "the tick is gone, the click plays");
    }

    #[test]
    fn mashing_cannot_queue_a_backlog() {
        let (mut p, t0) = (Player::default(), Instant::now());
        for i in 0..100 {
            p.start(Cue::Tap, None, t0 + ms(i));
        }
        let played = drain(&mut p, t0 + ms(100), 1000);
        assert_eq!(played.len(), 1, "only the last accepted tap is pending");
    }

    #[test]
    fn rumble_repeats_until_stopped() {
        let (mut p, t0) = (Player::default(), Instant::now());
        p.start(Cue::Rumble, None, t0);
        assert_eq!(drain(&mut p, t0, 280).len(), 4);
        assert!(p.stop());
        assert_eq!(p.poll(t0 + ms(300)), None);
    }

    #[test]
    fn rumble_stops_by_itself_at_its_cap() {
        let (mut p, t0) = (Player::default(), Instant::now());
        p.start(Cue::Rumble, None, t0);
        assert!(p.poll(t0 + ms(3900)).is_some());
        assert_eq!(p.poll(t0 + ms(4100)), None);
        assert!(p.next_due().is_none());
    }

    #[test]
    fn stop_leaves_a_confirm_alone() {
        let (mut p, t0) = (Player::default(), Instant::now());
        p.start(Cue::Success, None, t0);
        assert!(!p.stop());
        assert_eq!(drain(&mut p, t0, 200).len(), 2);
    }

    #[test]
    fn error_beats_a_rumble_and_a_rumble_does_not_beat_it() {
        let (mut p, t0) = (Player::default(), Instant::now());
        p.start(Cue::Rumble, None, t0);
        assert_eq!(p.start(Cue::Error, None, t0 + ms(10)), Start::Playing);
        assert_eq!(p.start(Cue::Rumble, None, t0 + ms(20)), Start::Dropped);
    }

    #[test]
    fn ramp_climbs() {
        let (mut p, t0) = (Player::default(), Instant::now());
        p.start(Cue::HoldRamp, None, t0);
        let ids: Vec<u8> = drain(&mut p, t0, 1000).into_iter().map(|(_, id)| id).collect();
        assert_eq!(ids, vec![3, 3, 2, 2, 1, 1, 1, 1]);
    }
}
