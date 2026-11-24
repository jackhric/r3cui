//! Haptic and audio feedback. The rest of the UI calls `play(Cue::…)`; a worker thread owns the
//! motor and the speaker, so nothing here blocks the caller. Patterns are data (see `Cue::spec`),
//! and `Player` is the pure scheduler behind them, driven with an explicit clock.
//!
//! The firmware gives the UI a control file for the motor in `UC_HAPTIC_DEV_PATH`; writing an
//! effect number to it as decimal text plays that effect. The numbers are the DRV2605 ROM
//! waveform ids (see docs/haptics.md), unverified except 1, 14 and 25.

use crate::audio::{self, Sound};
use crate::settings;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strength {
    Soft,
    Medium,
    Strong,
}

impl Strength {
    pub const ALL: [Strength; 3] = [Self::Soft, Self::Medium, Self::Strong];

    fn index(self) -> i8 {
        self as i8
    }

    fn shifted(self, by: i8) -> Self {
        Self::ALL[(self.index() + by).clamp(0, 2) as usize]
    }

    pub fn name(self) -> &'static str {
        ["Soft", "Medium", "Strong"][self.index() as usize]
    }
}

/// A motor effect: a family of ROM waveforms, one per strength.
#[derive(Clone, Copy, Debug)]
pub enum Effect {
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
    pub fn id(self, strength: Strength) -> u8 {
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
pub struct Step {
    pub effect: Effect,
    pub boost: i8,
    pub after_ms: u32,
}

macro_rules! s {
    ($effect:expr, $boost:expr, $after_ms:expr) => {
        Step { effect: $effect, boost: $boost, after_ms: $after_ms }
    };
}

#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub steps: &'static [Step],
    /// Run until `stop()`, or `max_ms`.
    pub repeat: bool,
    pub strength: Strength,
    /// A cue never interrupts a busier one of higher priority.
    pub priority: u8,
    /// Dropped, not replaced, when the motor is still busy with something of the same priority.
    pub drop_if_busy: bool,
    /// Requests closer together than this to the last one played of the same cue are ignored.
    pub min_gap_ms: u64,
    pub max_ms: u64,
    pub sound: Option<Sound>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    Tap,
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
    pub const ALL: [Cue; 16] = [
        Self::Tap,
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

    pub fn name(self) -> &'static str {
        match self {
            Self::Tap => "Tap",
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
    pub fn spec(self) -> Spec {
        use Effect::*;
        use Strength::*;
        const fn base(steps: &'static [Step], strength: Strength) -> Spec {
            Spec { steps, repeat: false, strength, priority: 1, drop_if_busy: false, min_gap_ms: 30, max_ms: 0, sound: None }
        }
        match self {
            Self::Tap => base(&[s!(Click, 0, 35)], Medium),
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
                max_ms: 3000,
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
            Self::Rumble => Spec { repeat: true, priority: 2, max_ms: 4000, min_gap_ms: 0, ..base(&[s!(Tick, 0, 90)], Medium) },
            Self::Alert => base(&[s!(Alert, 0, 800)], Medium),
            Self::LongBuzz => base(&[s!(LongBuzz, 0, 1000)], Medium),
            Self::RampUp => base(&[s!(RampUp, 0, 500)], Medium),
            Self::RampDown => base(&[s!(RampDown, 0, 500)], Medium),
            Self::Triple => base(&[s!(Click, 0, 60), s!(Click, 0, 60), s!(Click, 0, 40)], Medium),
            Self::Double => base(&[s!(DoubleClick, 0, 200)], Medium),
        }
    }

    /// `stop()` ends these early; short one-shots always play out.
    fn stoppable(self) -> bool {
        let spec = self.spec();
        spec.repeat || spec.max_ms > 0
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }
}

/// What the screen's events play. `None` stops whatever hold feedback is running.
pub const UI_EVENTS: &[(&str, Option<Cue>)] = &[
    ("tap", Some(Cue::Tap)),
    ("tick", Some(Cue::Tick)),
    ("select", Some(Cue::Select)),
    ("back", Some(Cue::Back)),
    ("success", Some(Cue::Success)),
    ("error", Some(Cue::Error)),
    ("power-hold", Some(Cue::Rumble)),
    ("power-complete", Some(Cue::Success)),
    ("home-hold", Some(Cue::HoldRamp)),
    ("home-complete", Some(Cue::HoldComplete)),
    ("hold-cancel", None),
];

pub const SLIDER_MUTE: Cue = Cue::Select;
pub const SLIDER_LIMIT: Cue = Cue::Limit;

pub fn from_ui(name: &str) -> Option<Option<Cue>> {
    UI_EVENTS.iter().find(|(n, _)| *n == name).map(|(_, cue)| *cue)
}

/// Run an event name from the UI.
pub fn ui_event(name: &str) {
    match from_ui(name) {
        Some(Some(cue)) => play(cue),
        Some(None) => stop(),
        None => tracing::debug!("unknown feedback event {name}"),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Start {
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
pub struct Player {
    active: Option<Active>,
    last: [Option<Instant>; Cue::ALL.len()],
    pub enabled: bool,
    /// 0..=1; lowers every strength by up to two notches.
    pub intensity: f32,
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
            i if i >= 0.67 => 0,
            i if i >= 0.34 => -1,
            _ => -2,
        })
    }

    pub fn start(&mut self, cue: Cue, strength: Option<Strength>, now: Instant) -> Start {
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
            ends: now + Duration::from_millis(if spec.max_ms > 0 { spec.max_ms } else { 60_000 }),
        });
        Start::Playing
    }

    /// Ends a rumble or ramp; a short one-shot such as a confirm is left to finish.
    pub fn stop(&mut self) -> bool {
        match &self.active {
            Some(a) if a.cue.stoppable() => {
                self.active = None;
                true
            }
            _ => false,
        }
    }

    pub fn cancel(&mut self) {
        self.active = None;
    }

    /// The effect to write now, if one is due.
    pub fn poll(&mut self, now: Instant) -> Option<u8> {
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
    pub fn next_due(&self) -> Option<Instant> {
        self.active.as_ref().map(|a| a.due.min(a.ends))
    }
}

enum Msg {
    Play(Cue, Option<Strength>),
    Stop,
    Raw(u8),
}

static CORE_HAPTICS: AtomicBool = AtomicBool::new(true);
static CORE_SOUND: AtomicBool = AtomicBool::new(true);
static CORE_VOLUME: AtomicU8 = AtomicU8::new(100);

/// The core's `get_haptic_cfg` / `get_sound_cfg`, as they arrive.
pub fn set_core_config(haptics: Option<bool>, sound: Option<bool>, volume: Option<u8>) {
    if let Some(on) = haptics {
        CORE_HAPTICS.store(on, Ordering::Relaxed);
    }
    if let Some(on) = sound {
        CORE_SOUND.store(on, Ordering::Relaxed);
    }
    if let Some(v) = volume {
        CORE_VOLUME.store(v.min(100), Ordering::Relaxed);
    }
}

fn sender() -> &'static Sender<Msg> {
    static TX: OnceLock<Sender<Msg>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new().name("feedback".into()).spawn(move || worker(rx)).ok();
        tx
    })
}

fn send(msg: Msg) {
    let _ = sender().send(msg);
}

pub fn play(cue: Cue) {
    send(Msg::Play(cue, None));
}

pub fn play_at(cue: Cue, strength: Strength) {
    send(Msg::Play(cue, Some(strength)));
}

pub fn stop() {
    send(Msg::Stop);
}

/// Writes one ROM effect number as is.
pub fn raw(id: u8) {
    send(Msg::Raw(id));
}

fn write_effect(id: u8) {
    let Some(path) = std::env::var_os("UC_HAPTIC_DEV_PATH") else { return };
    if let Err(e) = std::fs::write(&path, id.to_string()) {
        tracing::debug!("haptic effect {id}: {e}");
    }
}

fn worker(rx: Receiver<Msg>) {
    let cfg = settings::get();
    let mut audio = audio::backend();
    let mut player = Player::default();
    loop {
        let msg = match player.next_due() {
            Some(due) => match rx.recv_timeout(due.saturating_duration_since(Instant::now())) {
                Ok(m) => Some(m),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            },
            None => match rx.recv() {
                Ok(m) => Some(m),
                Err(_) => return,
            },
        };
        // Everything queued up while a pattern played is handled in one go.
        for msg in msg.into_iter().chain(rx.try_iter()) {
            player.enabled = cfg.haptics_enabled.unwrap_or(true) && CORE_HAPTICS.load(Ordering::Relaxed);
            player.intensity = cfg.haptics_intensity.unwrap_or(1.0);
            match msg {
                Msg::Play(cue, strength) => {
                    let started = player.start(cue, strength, Instant::now());
                    tracing::debug!(?cue, ?started, "feedback");
                    let sound_on = cfg.sound_enabled.unwrap_or(true) && CORE_SOUND.load(Ordering::Relaxed);
                    if started != Start::Dropped && sound_on {
                        if let Some(sound) = cue.spec().sound {
                            let volume = cfg.sound_volume.unwrap_or(0.5) * f32::from(CORE_VOLUME.load(Ordering::Relaxed)) / 100.0;
                            audio.play(sound, volume);
                        }
                    }
                }
                Msg::Stop => {
                    player.stop();
                }
                Msg::Raw(id) => {
                    player.cancel();
                    write_effect(id);
                }
            }
        }
        while let Some(id) = player.poll(Instant::now()) {
            write_effect(id);
        }
    }
}

/// State behind the Feedback lab page.
pub struct Lab {
    strength: Option<Strength>,
    buzzing: bool,
    raw: u8,
    page: usize,
}

const LAB_CUES_PER_PAGE: usize = 6;
const LAB_FIXED_ROWS: usize = 3;

impl Default for Lab {
    fn default() -> Self {
        Self { strength: None, buzzing: false, raw: 1, page: 0 }
    }
}

impl Lab {
    fn pages() -> usize {
        Cue::ALL.len().div_ceil(LAB_CUES_PER_PAGE)
    }

    /// (label, value) per row.
    pub fn rows(&self) -> Vec<(String, String)> {
        let mut rows = vec![
            ("Strength".to_string(), self.strength.map_or("Cue default", Strength::name).to_string()),
            ("Constant buzz".to_string(), if self.buzzing { "Running" } else { "Stopped" }.to_string()),
            ("Raw effect".to_string(), self.raw.to_string()),
        ];
        let first = self.page * LAB_CUES_PER_PAGE;
        for cue in Cue::ALL.iter().skip(first).take(LAB_CUES_PER_PAGE) {
            rows.push((cue.name().to_string(), cue.spec().strength.name().to_string()));
        }
        rows.push(("More cues".to_string(), format!("{}/{}", self.page + 1, Self::pages())));
        rows
    }

    pub fn pick(&mut self, row: usize) {
        match row {
            0 => {
                self.strength = match self.strength {
                    None => Some(Strength::Soft),
                    Some(Strength::Soft) => Some(Strength::Medium),
                    Some(Strength::Medium) => Some(Strength::Strong),
                    Some(Strength::Strong) => None,
                };
            }
            1 => {
                self.buzzing = !self.buzzing;
                if self.buzzing {
                    self.play(Cue::Rumble);
                } else {
                    stop();
                }
            }
            2 => {
                raw(self.raw);
                self.raw = if self.raw >= 123 { 1 } else { self.raw + 1 };
            }
            n => {
                let cue = n - LAB_FIXED_ROWS;
                if cue < LAB_CUES_PER_PAGE {
                    if let Some(cue) = Cue::ALL.get(self.page * LAB_CUES_PER_PAGE + cue) {
                        self.play(*cue);
                    }
                } else {
                    self.page = (self.page + 1) % Self::pages();
                }
            }
        }
    }

    fn play(&self, cue: Cue) {
        match self.strength {
            Some(s) => play_at(cue, s),
            None => play(cue),
        }
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

    #[test]
    fn ui_events_resolve() {
        assert_eq!(from_ui("power-hold"), Some(Some(Cue::Rumble)));
        assert_eq!(from_ui("hold-cancel"), Some(None));
        assert_eq!(from_ui("nope"), None);
    }

    #[test]
    fn lab_rows_page_through_the_cues() {
        let mut lab = Lab::default();
        let n = lab.rows().len();
        assert_eq!(n, LAB_FIXED_ROWS + LAB_CUES_PER_PAGE + 1);
        lab.pick(0);
        assert_eq!(lab.rows()[0].1, "Soft");
        lab.pick(n - 1);
        assert_eq!(lab.rows()[LAB_FIXED_ROWS].0, Cue::ALL[LAB_CUES_PER_PAGE].name());
    }
}
