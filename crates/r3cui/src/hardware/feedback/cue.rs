use crate::hardware::audio::Sound;
use crate::Haptic;

const DEFAULT_MIN_GAP_MS: u64 = 30;
const HOLD_RAMP_MAX_MS: u64 = 3000;
const RUMBLE_MAX_MS: u64 = 4000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Strength {
    Soft,
    Medium,
    Strong,
}

impl Strength {
    pub(super) const ALL: [Strength; 3] = [Self::Soft, Self::Medium, Self::Strong];

    fn index(self) -> i8 {
        self as i8
    }

    pub(super) fn shifted(self, by: i8) -> Self {
        Self::ALL[(self.index() + by).clamp(0, 2) as usize]
    }

    pub(super) fn name(self) -> &'static str {
        ["Soft", "Medium", "Strong"][self.index() as usize]
    }
}

/// A motor effect: a family of ROM waveforms, one per strength.
#[derive(Clone, Copy, Debug)]
pub(super) enum Effect {
    Click,
    Sharp,
    Tick,
    Bump,
    DoubleClick,
    Buzz,
    /// 750 ms alert.
    Alert,
    /// "Long buzz for programmatic stopping"; may need a stop write.
    LongBuzz,
    RampUp,
    RampDown,
}

impl Effect {
    pub(super) fn id(self, strength: Strength) -> u8 {
        let by_strength = |soft, medium, strong| [soft, medium, strong][strength.index() as usize];
        match self {
            Self::Click => by_strength(3, 2, 1),
            Self::Sharp => by_strength(6, 5, 4),
            Self::Tick => by_strength(26, 25, 24),
            Self::Bump => by_strength(9, 8, 7),
            Self::DoubleClick => by_strength(30, 28, 27),
            Self::Buzz => by_strength(50, 48, 14),
            Self::Alert => 15,
            Self::LongBuzz => 118,
            Self::RampUp => by_strength(86, 84, 82),
            Self::RampDown => by_strength(74, 72, 70),
        }
    }
}

/// One effect in a pattern. `boost` shifts the strength the pattern was played at (for ramps);
/// `after_ms` is how long to wait before the next step, which for the last step is how long the
/// motor stays busy. The motor reports no durations, so these are tuned by feel.
#[derive(Clone, Copy, Debug)]
pub(super) struct Step {
    pub(super) effect: Effect,
    pub(super) boost: i8,
    pub(super) after_ms: u32,
}

macro_rules! s {
    ($effect:expr, $boost:expr, $after_ms:expr) => {
        Step { effect: $effect, boost: $boost, after_ms: $after_ms }
    };
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Spec {
    pub(super) steps: &'static [Step],
    /// Run until `stop()`, or `max_ms`.
    pub(super) repeat: bool,
    pub(super) strength: Strength,
    /// A cue never interrupts a busier one of higher priority.
    pub(super) priority: u8,
    /// Dropped, not replaced, when the motor is still busy with something of the same priority.
    pub(super) drop_if_busy: bool,
    /// Requests closer together than this to the last one played of the same cue are ignored.
    pub(super) min_gap_ms: u64,
    pub(super) max_ms: u64,
    pub(super) sound: Option<Sound>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cue {
    Tap,
    Volume,
    Select,
    Back,
    Tick,
    Success,
    Error,
    Limit,
    HoldRamp,
    HoldComplete,
    Rumble,
    Alert,
    LongBuzz,
    RampUp,
    RampDown,
    Triple,
    Double,
}

impl Cue {
    pub(super) const ALL: [Cue; 17] = [
        Self::Tap,
        Self::Volume,
        Self::Select,
        Self::Back,
        Self::Tick,
        Self::Success,
        Self::Error,
        Self::Limit,
        Self::HoldRamp,
        Self::HoldComplete,
        Self::Rumble,
        Self::Alert,
        Self::LongBuzz,
        Self::RampUp,
        Self::RampDown,
        Self::Triple,
        Self::Double,
    ];

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Tap => "Tap",
            Self::Volume => "Volume",
            Self::Select => "Select",
            Self::Back => "Back",
            Self::Tick => "Tick",
            Self::Success => "Success",
            Self::Error => "Error",
            Self::Limit => "Limit bump",
            Self::HoldRamp => "Hold ramp-up",
            Self::HoldComplete => "Hold complete",
            Self::Rumble => "Rumble",
            Self::Alert => "Alert 750 ms",
            Self::LongBuzz => "Long buzz",
            Self::RampUp => "Ramp up",
            Self::RampDown => "Ramp down",
            Self::Triple => "Triple click",
            Self::Double => "Double click",
        }
    }

    /// The pattern and policy for each cue: edit this to tune how things feel.
    pub(super) fn spec(self) -> Spec {
        use Effect::*;
        use Strength::*;
        const fn base(steps: &'static [Step], strength: Strength) -> Spec {
            Spec { steps, repeat: false, strength, priority: 1, drop_if_busy: false, min_gap_ms: DEFAULT_MIN_GAP_MS, max_ms: 0, sound: None }
        }
        match self {
            Self::Tap => base(&[s!(Click, 0, 35)], Medium),
            // Effect 9: a soft bump, chosen by feel. Steps come fast, so each replaces the last.
            Self::Volume => Spec { min_gap_ms: 25, ..base(&[s!(Bump, 0, 30)], Soft) },
            Self::Select => Spec { priority: 2, sound: Some(Sound::Confirm), ..base(&[s!(Click, 0, 40)], Strong) },
            Self::Back => base(&[s!(Sharp, 0, 35)], Medium),
            Self::Tick => Spec { priority: 0, drop_if_busy: true, min_gap_ms: 50, ..base(&[s!(Tick, 0, 30)], Soft) },
            Self::Success => Spec {
                priority: 2,
                sound: Some(Sound::Success),
                ..base(&[s!(Tick, 0, 110), s!(Tick, 0, 40)], Medium)
            },
            Self::Error => Spec {
                priority: 3,
                sound: Some(Sound::Error),
                ..base(&[s!(Click, 0, 80), s!(Click, 0, 80), s!(Click, 0, 40)], Strong)
            },
            Self::Limit => Spec { drop_if_busy: true, min_gap_ms: 350, ..base(&[s!(Bump, 0, 90)], Strong) },
            Self::HoldRamp => Spec {
                priority: 2,
                max_ms: HOLD_RAMP_MAX_MS,
                ..base(
                    &[
                        s!(Click, -2, 80),
                        s!(Click, -2, 80),
                        s!(Click, -1, 80),
                        s!(Click, -1, 80),
                        s!(Click, 0, 80),
                        s!(Click, 0, 80),
                        s!(Click, 0, 80),
                        s!(Click, 0, 80),
                    ],
                    Strong,
                )
            },
            Self::HoldComplete => Spec {
                priority: 3,
                sound: Some(Sound::Success),
                ..base(&[s!(Buzz, 0, 120), s!(Click, 0, 60)], Strong)
            },
            Self::Rumble => Spec { repeat: true, priority: 2, max_ms: RUMBLE_MAX_MS, min_gap_ms: 0, ..base(&[s!(Tick, 0, 90)], Medium) },
            Self::Alert => base(&[s!(Alert, 0, 800)], Medium),
            Self::LongBuzz => base(&[s!(LongBuzz, 0, 1000)], Medium),
            Self::RampUp => base(&[s!(RampUp, 0, 500)], Medium),
            Self::RampDown => base(&[s!(RampDown, 0, 500)], Medium),
            Self::Triple => base(&[s!(Click, 0, 60), s!(Click, 0, 60), s!(Click, 0, 40)], Medium),
            Self::Double => base(&[s!(DoubleClick, 0, 200)], Medium),
        }
    }

    /// `stop()` ends these early; short one-shots always play out.
    pub(super) fn stoppable(self) -> bool {
        let spec = self.spec();
        spec.repeat || spec.max_ms > 0
    }

    pub(super) fn index(self) -> usize {
        Self::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }
}


/// What the screen's events play. `None` stops whatever hold feedback is running.
pub(super) fn cue_for(event: Haptic) -> Option<Cue> {
    match event {
        Haptic::Page => Some(Cue::Tap),
        Haptic::Select => Some(Cue::Select),
        Haptic::PowerHold => Some(Cue::Rumble),
        Haptic::PowerComplete => Some(Cue::Success),
        Haptic::HomeHold => Some(Cue::HoldRamp),
        Haptic::HomeComplete => Some(Cue::HoldComplete),
        Haptic::HoldCancel => None,
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_events_resolve() {
        assert_eq!(cue_for(Haptic::PowerHold), Some(Cue::Rumble));
        assert_eq!(cue_for(Haptic::HoldCancel), None);
    }
}
