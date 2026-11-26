//! The voice session on screen: its words, and the aurora that rises while it listens.

use crate::app::meter;
use crate::view::aurora::{self, Look};
use crate::view::stage::capture;
use crate::{AppWindow, Capture, Theme, Voice as Shown, VoiceStage};
use drivers::api::{Voice, VoiceFailure, VoicePhase};
use slint::ComponentHandle;
use std::cell::RefCell;
use std::time::{Duration, Instant};

const FRAME: Duration = Duration::from_millis(33);
/// The aurora comes in this fast, easing out.
const ENTER: Duration = Duration::from_millis(900);
/// How quickly it follows the voice getting louder, and quieter.
const ATTACK: Duration = Duration::from_millis(50);
const RELEASE: Duration = Duration::from_millis(280);
/// How quickly it settles once the voice has ended.
const SETTLE: Duration = Duration::from_millis(600);

pub(crate) fn show(app: &AppWindow, voice: Option<&Voice>) {
    let v = app.global::<Shown>();
    let Some(voice) = voice else {
        let stage = v.get_stage();
        if stage != VoiceStage::Off {
            v.set_cut_short(matches!(stage, VoiceStage::Listening | VoiceStage::Thinking));
            v.set_stage(VoiceStage::Off);
        }
        return;
    };
    let (stage, answer) = match voice.phase {
        VoicePhase::Listening => (VoiceStage::Listening, String::new()),
        VoicePhase::Thinking => (VoiceStage::Thinking, String::new()),
        VoicePhase::Answered { .. } => (VoiceStage::Answered, voice.answer.clone()),
        VoicePhase::Failed(f) => (VoiceStage::Failed, failure(f).to_string()),
    };
    // Opening, the screen under it is blurred as it is for every overlay.
    if v.get_stage() == VoiceStage::Off {
        capture(app, Capture::World);
    }
    v.set_stage(stage);
    v.set_heard(voice.heard.as_str().into());
    v.set_answer(answer.into());
}

fn failure(f: VoiceFailure) -> &'static str {
    match f {
        VoiceFailure::NoTextRecognized => "Didn't catch that",
        VoiceFailure::Timeout => "No answer in time",
        VoiceFailure::ServiceUnavailable => "Voice isn't reachable",
        VoiceFailure::InvalidAudio => "The audio didn't come through",
        VoiceFailure::Unexpected => "Something went wrong",
    }
}

/// One session's aurora as it moves.
struct Scene {
    look: Look,
    began: Instant,
    last: Instant,
}

/// Moves `cur` toward `target`, at `attack` rising and `release` falling.
fn follow(cur: f32, target: f32, dt: f32, attack: Duration, release: Duration) -> f32 {
    let tau = if target > cur { attack } else { release }.as_secs_f32();
    cur + (target - cur) * (1.0 - (-dt / tau).exp())
}

/// Redraws the aurora while the voice overlay is up; each session gets its own seed.
pub(crate) fn animate_aurora(app: &AppWindow) -> slint::Timer {
    let weak = app.as_weak();
    let scene: RefCell<Option<Scene>> = RefCell::new(None);
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, FRAME, move || {
        let Some(app) = weak.upgrade() else { return };
        let stage = app.global::<Shown>().get_stage();
        let mut scene = scene.borrow_mut();
        if stage == VoiceStage::Off {
            *scene = None;
            return;
        }
        let now = Instant::now();
        let s = scene.get_or_insert_with(|| {
            let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos());
            Scene { look: Look { seed, flow: 0.0, reach: 0.0, level: 0.0, bands: [0.0; 4], dim: 0.0 }, began: now, last: now }
        });
        let dt = now.duration_since(s.last).as_secs_f32();
        s.last = now;
        let listening = stage == VoiceStage::Listening;
        let heard = if listening { meter::latest() } else { meter::Heard::default() };
        let l = &mut s.look;
        l.level = follow(l.level, heard.level, dt, ATTACK, RELEASE);
        for (b, target) in l.bands.iter_mut().zip(heard.bands) {
            *b = follow(*b, target, dt, ATTACK, RELEASE);
        }
        l.dim = follow(l.dim, if listening { 0.0 } else { 1.0 }, dt, SETTLE, SETTLE);
        let t = (now.duration_since(s.began).as_secs_f32() / ENTER.as_secs_f32()).min(1.0);
        l.reach = 1.0 - (1.0 - t).powi(3);
        l.flow += dt * (1.0 + 3.0 * l.level) * (1.0 - 0.6 * l.dim);
        let a = app.global::<Theme>().get_accent();
        let pixels = aurora::render(l, &aurora::palette((a.red(), a.green(), a.blue())));
        app.global::<Shown>().set_aurora(slint::Image::from_rgba8(pixels));
    });
    timer
}
