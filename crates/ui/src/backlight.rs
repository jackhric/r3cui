//! Key backlight engine.
//!
//! What the hardware does (found on the device, not documented):
//! - The keys are lit in 12 zones, named `ZONE_1`..`ZONE_12`. Unknown names are accepted and ignored.
//! - An update (`set_button_cfg` → `static_color: {rgb, zones}`) paints only the zones it names.
//!   Every other zone keeps what it had, so each zone can hold its own colour, and turning a zone
//!   off means painting it black.
//! - Some zones share keys, and light from a zone bleeds into its neighbours' keycaps.
//!
//! How to use it:
//!
//! ```ignore
//! let keys = Backlight::start(core);                       // once per core connection
//! keys.show(Frame::OFF.light(DPAD, accent).light(&[Zone::ChannelUpPower], Rgb::GREEN));
//! ```
//!
//! A [`Frame`] is the whole keypad: one colour per zone. [`Backlight::show`] can be called as often
//! as you like; only the zones that differ from what is already showing are repainted, grouped
//! into one update per colour, and a newer frame replaces one that has not been sent yet.

use crate::{AppWindow, Theme};
use serde_json::json;
use slint::ComponentHandle;
use std::sync::Arc;
use tokio::sync::watch;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const OFF: Rgb = Rgb(0, 0, 0);
    pub const RED: Rgb = Rgb(0xE8, 0x34, 0x3A);
    pub const GREEN: Rgb = Rgb(0x3D, 0xDC, 0x84);

    /// The same hue at a fraction of the brightness.
    pub fn dim(self, fraction: f32) -> Rgb {
        let f = |c: u8| (c as f32 * fraction.clamp(0.0, 1.0)).round() as u8;
        Rgb(f(self.0), f(self.1), f(self.2))
    }

    pub fn is_off(self) -> bool {
        self == Rgb::OFF
    }
}

/// The zones as the UCR3 reports them in `/cfg/device/button_layout`; several cover two or more keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Zone {
    Home = 1,
    VolumeUpBack,
    DpadUp,
    ChannelUpPower,
    DpadLeft,
    DpadRight,
    DpadDown,
    VolumeDown,
    ChannelDown,
    MutePrevious,
    RecordMenuStopPlay,
    VoiceNext,
}

impl Zone {
    pub const ALL: [Zone; 12] = [
        Zone::Home,
        Zone::VolumeUpBack,
        Zone::DpadUp,
        Zone::ChannelUpPower,
        Zone::DpadLeft,
        Zone::DpadRight,
        Zone::DpadDown,
        Zone::VolumeDown,
        Zone::ChannelDown,
        Zone::MutePrevious,
        Zone::RecordMenuStopPlay,
        Zone::VoiceNext,
    ];

    pub fn index(self) -> usize {
        self as usize - 1
    }

    fn core_name(self) -> String {
        format!("ZONE_{}", self as u8)
    }
}

pub const DPAD: &[Zone] = &[Zone::DpadUp, Zone::DpadLeft, Zone::DpadRight, Zone::DpadDown];
pub const VOLUME: &[Zone] = &[Zone::VolumeUpBack, Zone::VolumeDown];
pub const TRANSPORT: &[Zone] = &[Zone::MutePrevious, Zone::RecordMenuStopPlay, Zone::VoiceNext];

/// Pairs of zones whose light visibly spills into each other's keys. A lit zone next to a dark one
/// looks like a fault, so [`Frame::spill`] gives the dark neighbour a faint share of the colour.
/// Fill this in from what is seen on the device.
const BLEED: &[(Zone, Zone)] = &[];

/// How bright a dark neighbour is made, as a fraction of the lit zone beside it.
const SPILL: f32 = 0.18;

/// The whole keypad: one colour per zone, black for off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame([Rgb; 12]);

impl Frame {
    pub const OFF: Frame = Frame([Rgb::OFF; 12]);

    /// Paint these zones; later calls win where they overlap.
    pub fn light(mut self, zones: &[Zone], colour: Rgb) -> Self {
        for z in zones {
            self.0[z.index()] = colour;
        }
        self
    }

    pub fn get(&self, zone: Zone) -> Rgb {
        self.0[zone.index()]
    }

    /// Give each dark zone that sits beside a lit one a faint share of its colour (see [`BLEED`]).
    pub fn spill(self) -> Self {
        let mut out = self;
        for &(a, b) in BLEED {
            for (lit, dark) in [(a, b), (b, a)] {
                if !self.get(lit).is_off() && self.get(dark).is_off() {
                    out.0[dark.index()] = self.get(lit).dim(SPILL);
                }
            }
        }
        out
    }
}

/// The updates that turn `shown` into `want`: only the zones that differ, one update per colour,
/// black first so nothing is briefly lit that should not be. `None` repaints everything.
fn plan(shown: Option<&Frame>, want: &Frame) -> Vec<(Rgb, Vec<Zone>)> {
    let mut groups: Vec<(Rgb, Vec<Zone>)> = Vec::new();
    for zone in Zone::ALL {
        let colour = want.get(zone);
        if shown.is_some_and(|s| s.get(zone) == colour) {
            continue;
        }
        match groups.iter_mut().find(|(c, _)| *c == colour) {
            Some((_, zones)) => zones.push(zone),
            None => groups.push((colour, vec![zone])),
        }
    }
    groups.sort_by_key(|(colour, _)| !colour.is_off());
    groups
}

/// Handle to the task that keeps the remote's keys showing the latest frame.
#[derive(Clone)]
pub struct Backlight(Arc<watch::Sender<Frame>>);

impl Backlight {
    pub fn start(core: coreapi::Core) -> Self {
        let (tx, mut wanted) = watch::channel(Frame::OFF);
        tokio::spawn(async move {
            // Nothing is known about the keys at first, so the first frame paints every zone.
            let mut shown: Option<Frame> = None;
            while wanted.changed().await.is_ok() {
                let want = *wanted.borrow_and_update();
                let mut ok = true;
                for (colour, zones) in plan(shown.as_ref(), &want) {
                    let names: Vec<String> = zones.iter().map(|z| z.core_name()).collect();
                    let cfg = json!({ "static_color": { "rgb": [colour.0, colour.1, colour.2], "zones": names } });
                    match core.request("set_button_cfg", cfg).await {
                        Ok(r) if (200..300).contains(&r.code) => {}
                        other => {
                            tracing::debug!("backlight: {other:?}");
                            ok = false;
                        }
                    }
                }
                // After a failure the keys are in an unknown state; repaint in full next time.
                shown = ok.then_some(want);
            }
        });
        Self(Arc::new(tx))
    }

    pub fn show(&self, frame: Frame) {
        let _ = self.0.send(frame);
    }
}

/// What decides the keys: a snapshot of what the window is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiState {
    pub awake: bool,
    /// While the power key is held: `Some(true)` turning off, `Some(false)` turning on.
    pub power_hold: Option<bool>,
    pub receiver_on: bool,
    /// 0 home, anything else is a list page (inputs, menu).
    pub page: i32,
    pub kodi: bool,
    pub playing: bool,
    pub volume_overlay: bool,
    pub muted: bool,
}

/// UI thread only.
pub fn observe(app: &AppWindow) -> (UiState, Rgb) {
    let accent = app.global::<Theme>().get_accent();
    let state = UiState {
        awake: app.get_awake(),
        power_hold: app.get_power_overlay().then(|| app.get_power_turning_off()),
        receiver_on: app.get_receiver_on(),
        page: app.get_page(),
        kodi: app.get_kodi(),
        playing: !app.get_media_title().is_empty(),
        volume_overlay: app.get_volume_overlay(),
        muted: app.get_muted(),
    };
    (state, Rgb(accent.red(), accent.green(), accent.blue()))
}

/// The policy: which keys are lit, and in what colour, for what is on screen. Keys that do
/// something right now are lit in the accent; the power key shows red or green when it matters.
pub fn frame_for(s: UiState, accent: Rgb) -> Frame {
    if !s.awake {
        return Frame::OFF;
    }
    let volume = |frame: Frame| {
        if s.muted {
            // Muted: the mute key is the one to press; volume still works but steps back.
            frame.light(VOLUME, accent.dim(0.25)).light(&[Zone::MutePrevious], accent)
        } else {
            frame.light(VOLUME, accent).light(&[Zone::MutePrevious], accent)
        }
    };
    let base = if s.page != 0 {
        Frame::OFF.light(DPAD, accent).light(&[Zone::Home, Zone::VolumeUpBack], accent)
    } else if !s.receiver_on {
        // Everything is off; the power key is the only way forward.
        Frame::OFF.light(&[Zone::ChannelUpPower], Rgb::GREEN.dim(0.4))
    } else if s.volume_overlay || !s.kodi {
        volume(Frame::OFF)
    } else if s.playing {
        volume(Frame::OFF.light(TRANSPORT, accent))
    } else {
        volume(Frame::OFF.light(DPAD, accent).light(&[Zone::Home, Zone::RecordMenuStopPlay], accent))
    };
    let frame = match s.power_hold {
        Some(turning_off) => base.light(&[Zone::ChannelUpPower], if turning_off { Rgb::RED } else { Rgb::GREEN }),
        None => base,
    };
    frame.spill()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCENT: Rgb = Rgb(10, 20, 30);

    fn state() -> UiState {
        UiState { awake: true, power_hold: None, receiver_on: true, page: 0, kodi: false, playing: false, volume_overlay: false, muted: false }
    }

    #[test]
    fn first_frame_paints_every_zone_black_first() {
        let want = Frame::OFF.light(DPAD, ACCENT);
        let updates = plan(None, &want);
        assert_eq!(updates.len(), 2);
        assert_eq!((updates[0].0, updates[0].1.len()), (Rgb::OFF, 8));
        assert_eq!((updates[1].0, updates[1].1.as_slice()), (ACCENT, DPAD));
    }

    #[test]
    fn only_changed_zones_are_repainted() {
        let shown = Frame::OFF.light(DPAD, ACCENT);
        let want = shown.light(&[Zone::Home], Rgb::GREEN).light(&[Zone::DpadUp], Rgb::OFF);
        assert_eq!(plan(Some(&shown), &want), vec![(Rgb::OFF, vec![Zone::DpadUp]), (Rgb::GREEN, vec![Zone::Home])]);
        assert!(plan(Some(&want), &want).is_empty());
    }

    #[test]
    fn asleep_is_dark_and_the_power_hold_is_coloured() {
        assert_eq!(frame_for(UiState { awake: false, ..state() }, ACCENT), Frame::OFF);
        let holding = frame_for(UiState { power_hold: Some(true), ..state() }, ACCENT);
        assert_eq!(holding.get(Zone::ChannelUpPower), Rgb::RED);
        assert_eq!(holding.get(Zone::VolumeDown), ACCENT, "the rest of the keys keep their colours");
    }

    #[test]
    fn muted_dims_volume_but_not_mute() {
        let f = frame_for(UiState { muted: true, ..state() }, ACCENT);
        assert_eq!(f.get(Zone::MutePrevious), ACCENT);
        assert_eq!(f.get(Zone::VolumeDown), ACCENT.dim(0.25));
    }
}
