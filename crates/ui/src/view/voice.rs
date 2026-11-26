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
/// The aurora's entrance, each part easing out: it spreads from the bottom centre along the edge, and from a moment
/// after it begins, climbs the sides.
const SPREAD: Duration = Duration::from_millis(450);
const CLIMB_AFTER: Duration = Duration::from_millis(250);
const CLIMB: Duration = Duration::from_millis(650);
/// How quickly it follows the voice getting louder, and quieter.
const ATTACK: Duration = Duration::from_millis(50);
const RELEASE: Duration = Duration::from_millis(280);
/// How quickly it settles once the voice has ended.
const SETTLE: Duration = Duration::from_millis(600);
/// Set (in `settings.json`'s `env`, say) to time the aurora once at startup, for a device without a voice to hand.
const BENCH_ENV: &str = "R3CUI_AURORA_BENCH";
const BENCH_FRAMES: u32 = 90;
/// While it runs, how often the frame times are logged.
const REPORT_EVERY: u32 = 150;

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

/// Draws aurora frames off the UI thread, always the newest asked for, and hands each to the window.
fn painter(weak: slint::Weak<AppWindow>) -> std::sync::mpsc::Sender<(Look, [[f32; 3]; 4])> {
    let (tx, rx) = std::sync::mpsc::channel::<(Look, [[f32; 3]; 4])>();
    std::thread::spawn(move || {
        let (mut frames, mut total, mut longest) = (0u32, Duration::ZERO, Duration::ZERO);
        while let Ok(mut job) = rx.recv() {
            // A slow frame is not worth drawing once a newer one is waiting.
            while let Ok(newer) = rx.try_recv() {
                job = newer;
            }
            let start = Instant::now();
            let pixels = aurora::render(&job.0, &job.1);
            let took = start.elapsed();
            (frames, total, longest) = (frames + 1, total + took, longest.max(took));
            if frames == REPORT_EVERY {
                tracing::info!("aurora: {frames} frames, mean {:.2} ms, longest {:.2} ms", total.as_secs_f64() * 1000.0 / f64::from(frames), longest.as_secs_f64() * 1000.0);
                (frames, total, longest) = (0, Duration::ZERO, Duration::ZERO);
            }
            let _ = weak.upgrade_in_event_loop(move |app| app.global::<Shown>().set_aurora(slint::Image::from_rgba8(pixels)));
        }
    });
    tx
}

/// Times the aurora off the UI thread, at full voice, when `R3CUI_AURORA_BENCH` is set.
pub(crate) fn bench_aurora() {
    if std::env::var_os(BENCH_ENV).is_none() {
        return;
    }
    std::thread::spawn(|| {
        let palette = aurora::palette((0x2E, 0x7B, 0xFF));
        let (mut total, mut longest) = (Duration::ZERO, Duration::ZERO);
        for i in 0..BENCH_FRAMES {
            let x = i as f32 / BENCH_FRAMES as f32;
            let look = Look { seed: 7, flow: x * 4.0, spread: 1.0, reach: 1.0, level: 0.5 + 0.5 * (x * 20.0).sin(), bands: [0.5; 4], dim: 0.0 };
            let start = Instant::now();
            std::hint::black_box(aurora::render(&look, &palette));
            let took = start.elapsed();
            total += took;
            longest = longest.max(took);
        }
        tracing::info!("aurora bench: {}x{}, {BENCH_FRAMES} frames, mean {:.2} ms, longest {:.2} ms", aurora::W, aurora::H, total.as_secs_f64() * 1000.0 / f64::from(BENCH_FRAMES), longest.as_secs_f64() * 1000.0);
    });
}

/// Moves `cur` toward `target`, at `attack` rising and `release` falling.
fn follow(cur: f32, target: f32, dt: f32, attack: Duration, release: Duration) -> f32 {
    let tau = if target > cur { attack } else { release }.as_secs_f32();
    cur + (target - cur) * (1.0 - (-dt / tau).exp())
}

/// Redraws the aurora while the voice overlay is up; each session gets its own seed.
pub(crate) fn animate_aurora(app: &AppWindow) -> slint::Timer {
    let weak = app.as_weak();
    let paint = painter(app.as_weak());
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
            Scene { look: Look { seed, flow: 0.0, spread: 0.0, reach: 0.0, level: 0.0, bands: [0.0; 4], dim: 0.0 }, began: now, last: now }
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
        let since = now.duration_since(s.began);
        let ease_out = |part: f32| 1.0 - (1.0 - part.clamp(0.0, 1.0)).powi(3);
        l.spread = ease_out(since.as_secs_f32() / SPREAD.as_secs_f32());
        l.reach = ease_out(since.saturating_sub(CLIMB_AFTER).as_secs_f32() / CLIMB.as_secs_f32());
        l.flow += dt * (1.0 + 3.0 * l.level) * (1.0 - 0.6 * l.dim);
        let a = app.global::<Theme>().get_accent();
        let _ = paint.send((*l, aurora::palette((a.red(), a.green(), a.blue()))));
    });
    timer
}
