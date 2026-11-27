use super::cue::{Cue, Strength};
use super::device::write_effect;
use super::player::{Player, Start};
use crate::app::settings;
use crate::hardware::audio;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::OnceLock;
use std::time::Instant;

const WORKER_THREAD: &str = "feedback";
const FULL_VOLUME: u8 = 100;
const DEFAULT_SOUND_VOLUME: f32 = 0.5;

enum Msg {
    Play(Cue, Option<Strength>),
    Stop,
    Raw(u8),
}

static CORE_HAPTICS: AtomicBool = AtomicBool::new(true);
static CORE_SOUND: AtomicBool = AtomicBool::new(true);
static CORE_VOLUME: AtomicU8 = AtomicU8::new(FULL_VOLUME);

/// The core's `get_haptic_cfg` / `get_sound_cfg`, as they arrive.
pub(crate) fn set_core_config(haptics: Option<bool>, sound: Option<bool>, volume: Option<u8>) {
    if let Some(on) = haptics {
        CORE_HAPTICS.store(on, Ordering::Relaxed);
    }
    if let Some(on) = sound {
        CORE_SOUND.store(on, Ordering::Relaxed);
    }
    if let Some(v) = volume {
        CORE_VOLUME.store(v.min(FULL_VOLUME), Ordering::Relaxed);
    }
}

fn sender() -> &'static Sender<Msg> {
    static TX: OnceLock<Sender<Msg>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new().name(WORKER_THREAD.into()).spawn(move || worker(rx)).ok();
        tx
    })
}

fn send(msg: Msg) {
    let _ = sender().send(msg);
}

pub(crate) fn play(cue: Cue) {
    send(Msg::Play(cue, None));
}

pub(crate) fn play_at(cue: Cue, strength: Strength) {
    send(Msg::Play(cue, Some(strength)));
}

pub(crate) fn stop() {
    send(Msg::Stop);
}

/// Writes one ROM effect number as is.
pub(super) fn raw(id: u8) {
    send(Msg::Raw(id));
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
                            let volume = cfg.sound_volume.unwrap_or(DEFAULT_SOUND_VOLUME) * f32::from(CORE_VOLUME.load(Ordering::Relaxed)) / f32::from(FULL_VOLUME);
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
