//! Key backlight: the keypad in one colour, with the power key able to stand alone.
//!
//! The hardware lights the keys in 12 zones (`ZONE_1`..`ZONE_12`) that can each hold a colour, but
//! light bleeds badly between most of them, so they are treated as two groups: the power zone,
//! which is cleanly separate, and everything else.
//!
//! - Normally every key shows the interface accent, which follows the receiver's input.
//! - While the power key is held, only it is lit: red to turn off, green to turn on.
//! - With the receiver off, only the power key is lit, so it is clear what to hold.
//! - Home Assistant can replace the colour or switch the keys off ([`Ha`]).
//!
//! ```ignore
//! let keys = Backlight::start(core);                         // once per core connection
//! keys.show(Keys::all(Rgb(232, 52, 58)));                    // repeats are not re-sent
//! keys.show(Keys { power: Rgb::GREEN, rest: Rgb::OFF });
//! ```
//!
//! Things about the core's `set_button_cfg` worth knowing: it accepts unknown zone names without
//! lighting anything, and it only repaints the zones an update names. It also dims the keys by
//! itself when the remote idles, and on waking restores only the zones it was last given, so
//! every change of power mode calls for a full repaint ([`Backlight::repaint`]). It has no fade of
//! its own, so colour changes are crossfaded here by sending the colours in between ([`FADE`]).

use crate::{AppWindow, Theme};
use serde_json::json;
use slint::ComponentHandle;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;

/// How long the keys take to crossfade to a new colour.
pub const FADE: Duration = Duration::from_millis(320);
const FADE_STEPS: u32 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const OFF: Rgb = Rgb(0, 0, 0);
    pub const RED: Rgb = Rgb(0xE8, 0x34, 0x3A);
    pub const GREEN: Rgb = Rgb(0x3D, 0xDC, 0x84);

    fn mix(self, to: Rgb, t: f32) -> Rgb {
        let ch = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Rgb(ch(self.0, to.0), ch(self.1, to.1), ch(self.2, to.2))
    }
}

/// How many zones the UCR3 keypad has (`/cfg/device/button_layout`).
const ZONES: u8 = 12;
/// The zone under the power key. It also covers channel up.
const POWER_ZONE: u8 = 4;

/// The keypad: the power key's zone, and all the others together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keys {
    pub power: Rgb,
    pub rest: Rgb,
}

impl Keys {
    pub const OFF: Keys = Keys::all(Rgb::OFF);

    pub const fn all(colour: Rgb) -> Self {
        Self { power: colour, rest: colour }
    }

    /// Part way from `self` to `to`, eased in and out.
    pub(crate) fn mix(self, to: Keys, t: f32) -> Keys {
        let t = t * t * (3.0 - 2.0 * t);
        Keys { power: self.power.mix(to.power, t), rest: self.rest.mix(to.rest, t) }
    }
}

/// Handle to the task that keeps the remote's keys showing the latest request.
#[derive(Clone)]
pub struct Backlight(Arc<watch::Sender<(Keys, u64)>>);

impl Backlight {
    pub fn start(core: coreapi::Core) -> Self {
        let (tx, mut wanted) = watch::channel((Keys::OFF, 0u64));
        tokio::spawn(async move {
            // Nothing is known about the keys at first, so the first request paints both groups.
            let mut shown: Option<Keys> = None;
            let mut painted = 0u64;
            'requests: while wanted.changed().await.is_ok() {
                let (want, generation) = *wanted.borrow_and_update();
                if generation != painted {
                    painted = generation;
                    shown = None;
                }
                // From a known colour, step through the ones in between. A request that arrives
                // mid-fade starts its own fade from wherever this one got to.
                if let Some(from) = shown.filter(|s| *s != want) {
                    for i in 1..FADE_STEPS {
                        let step = from.mix(want, i as f32 / FADE_STEPS as f32);
                        if !paint_keys(&core, shown, step).await {
                            shown = None;
                            break;
                        }
                        shown = Some(step);
                        tokio::time::sleep(FADE / FADE_STEPS).await;
                        if wanted.has_changed().unwrap_or(false) {
                            continue 'requests;
                        }
                    }
                }
                // After a failure the keys are in an unknown state; repaint in full next time.
                shown = paint_keys(&core, shown, want).await.then_some(want);
            }
        });
        Self(Arc::new(tx))
    }

    pub fn show(&self, keys: Keys) {
        self.0.send_if_modified(|(k, _)| std::mem::replace(k, keys) != keys);
    }

    /// Paint every zone again, whatever the core is believed to show.
    pub fn repaint(&self) {
        self.0.send_modify(|(_, generation)| *generation += 1);
    }
}

/// Paints the groups whose colour differs from `shown` (all of them when it is unknown).
async fn paint_keys(core: &coreapi::Core, shown: Option<Keys>, keys: Keys) -> bool {
    let paint = |colour: Rgb, zones: Vec<u8>| {
        let names: Vec<String> = zones.iter().map(|z| format!("ZONE_{z}")).collect();
        let cfg = json!({ "static_color": { "rgb": [colour.0, colour.1, colour.2], "zones": names } });
        async move { matches!(core.request("set_button_cfg", cfg).await, Ok(r) if r.is_ok()) }
    };
    let mut ok = true;
    if shown.map(|s| s.rest) != Some(keys.rest) {
        ok &= paint(keys.rest, (1..=ZONES).filter(|z| *z != POWER_ZONE).collect()).await;
    }
    if shown.map(|s| s.power) != Some(keys.power) {
        ok &= paint(keys.power, vec![POWER_ZONE]).await;
    }
    ok
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// The colour follows the remote's own accent.
    #[default]
    Follow,
    /// Home Assistant's light decides the colour and brightness.
    Custom,
    Off,
}

/// What Home Assistant asks of the keys, read from its two helper entities.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ha {
    pub mode: Mode,
    /// The helper light's colour, already scaled by its brightness; `None` while it is off.
    pub colour: Option<Rgb>,
}

static HA: Mutex<Option<Ha>> = Mutex::new(None);

/// Record Home Assistant's latest wishes; the next request takes them into account.
pub fn set_ha(ha: Ha) {
    *HA.lock().unwrap_or_else(|e| e.into_inner()) = Some(ha);
}

/// What decides the keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiState {
    pub awake: bool,
    pub receiver_on: bool,
    /// While the power key is held: `Some(true)` turning off, `Some(false)` turning on.
    pub power_hold: Option<bool>,
    pub ha: Ha,
}

/// UI thread only.
pub fn observe(app: &AppWindow) -> (UiState, Rgb) {
    let accent = app.global::<Theme>().get_accent();
    let state = UiState {
        awake: app.get_lit(),
        receiver_on: app.get_receiver_on(),
        power_hold: app.get_power_overlay().then(|| app.get_power_turning_off()),
        ha: HA.lock().unwrap_or_else(|e| e.into_inner()).unwrap_or_default(),
    };
    (state, Rgb(accent.red(), accent.green(), accent.blue()))
}

/// The policy: what the keys show for the current state.
pub fn keys_for(s: UiState, accent: Rgb) -> Keys {
    if !s.awake || s.ha.mode == Mode::Off {
        return Keys::OFF;
    }
    match s.power_hold {
        Some(turning_off) => Keys { power: if turning_off { Rgb::RED } else { Rgb::GREEN }, rest: Rgb::OFF },
        // Everything is off, and holding power is the way back.
        None if !s.receiver_on => Keys { power: Rgb::GREEN, rest: Rgb::OFF },
        None => Keys::all(match (s.ha.mode, s.ha.colour) {
            (Mode::Custom, Some(colour)) => colour,
            _ => accent,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCENT: Rgb = Rgb(10, 20, 30);

    fn state() -> UiState {
        UiState { awake: true, receiver_on: true, power_hold: None, ha: Ha::default() }
    }

    #[test]
    fn crossfades_ease_between_the_two_colours() {
        let (from, to) = (Keys::all(Rgb(232, 52, 58)), Keys::all(Rgb(46, 123, 255)));
        assert_eq!(from.mix(to, 0.0), from);
        assert_eq!(from.mix(to, 1.0), to);
        assert_eq!(from.mix(to, 0.5).rest, Rgb(139, 88, 157));
        assert!(from.mix(to, 0.1).rest.0 > 220, "eased: slow to leave the first colour");
    }

    #[test]
    fn follows_the_accent_and_goes_dark_asleep() {
        assert_eq!(keys_for(state(), ACCENT), Keys::all(ACCENT));
        assert_eq!(keys_for(UiState { awake: false, ..state() }, ACCENT), Keys::OFF);
    }

    #[test]
    fn holding_power_lights_only_the_power_key() {
        let off = keys_for(UiState { power_hold: Some(true), ..state() }, ACCENT);
        assert_eq!(off, Keys { power: Rgb::RED, rest: Rgb::OFF });
        let on = keys_for(UiState { power_hold: Some(false), receiver_on: false, ..state() }, ACCENT);
        assert_eq!(on, Keys { power: Rgb::GREEN, rest: Rgb::OFF });
    }

    #[test]
    fn with_the_receiver_off_only_the_power_key_is_lit() {
        assert_eq!(keys_for(UiState { receiver_on: false, ..state() }, ACCENT), Keys { power: Rgb::GREEN, rest: Rgb::OFF });
    }

    #[test]
    fn home_assistant_can_override_or_switch_off() {
        let blue = Some(Rgb(0, 0, 200));
        let custom = Ha { mode: Mode::Custom, colour: blue };
        assert_eq!(keys_for(UiState { ha: custom, ..state() }, ACCENT), Keys::all(Rgb(0, 0, 200)));
        assert_eq!(keys_for(UiState { ha: custom, power_hold: Some(true), ..state() }, ACCENT).power, Rgb::RED);
        // Custom with the light off, or Follow with it on, both mean the remote's own colour.
        assert_eq!(keys_for(UiState { ha: Ha { mode: Mode::Custom, colour: None }, ..state() }, ACCENT), Keys::all(ACCENT));
        assert_eq!(keys_for(UiState { ha: Ha { mode: Mode::Follow, colour: blue }, ..state() }, ACCENT), Keys::all(ACCENT));
        let off = Ha { mode: Mode::Off, colour: blue };
        assert_eq!(keys_for(UiState { ha: off, power_hold: Some(true), ..state() }, ACCENT), Keys::OFF);
    }
}
