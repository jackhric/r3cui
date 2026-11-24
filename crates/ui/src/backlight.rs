//! Key backlight engine.
//!
//! What the hardware does (found on the device, not documented):
//! - The keys are lit in 12 zones, named `ZONE_1`..`ZONE_12`. Unknown names are accepted and ignored.
//! - An update (`set_button_cfg` → `static_color: {rgb, zones}`) paints only the zones it names.
//!   Every other zone keeps what it had, so each zone can hold its own colour, and turning a zone
//!   off means painting it black.
//! - Some zones share keys, and light from a zone bleeds into its neighbours' keycaps. A lit key
//!   beside a dark one therefore looks like a fault, so keys that are not in use are given a
//!   soft white ([`UNDERLAY`]) instead of being switched off.
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
//!
//! [`frame_for`] is the policy that turns what is on screen, plus any wishes from Home Assistant
//! ([`Ha`]), into a frame.

use crate::{AppWindow, Theme};
use serde_json::json;
use slint::ComponentHandle;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const OFF: Rgb = Rgb(0, 0, 0);
    pub const WHITE: Rgb = Rgb(255, 255, 255);
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

    fn peak(self) -> f32 {
        self.0.max(self.1).max(self.2) as f32
    }
}

/// What keys that are not in use show: soft enough to recede, bright enough to hide the bleed.
pub const UNDERLAY: Rgb = Rgb(77, 77, 77);

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

    /// How the zone is spelled at the end of its Home Assistant light's entity id.
    pub fn slug(self) -> &'static str {
        match self {
            Zone::Home => "home",
            Zone::VolumeUpBack => "volume_up_and_back",
            Zone::DpadUp => "d_pad_up",
            Zone::ChannelUpPower => "channel_up_and_power",
            Zone::DpadLeft => "d_pad_left",
            Zone::DpadRight => "d_pad_right",
            Zone::DpadDown => "d_pad_down",
            Zone::VolumeDown => "volume_down",
            Zone::ChannelDown => "channel_down",
            Zone::MutePrevious => "mute_and_previous",
            Zone::RecordMenuStopPlay => "record_menu_stop_play",
            Zone::VoiceNext => "voice_and_next",
        }
    }
}

pub const DPAD: &[Zone] = &[Zone::DpadUp, Zone::DpadLeft, Zone::DpadRight, Zone::DpadDown];
pub const VOLUME: &[Zone] = &[Zone::VolumeUpBack, Zone::VolumeDown];
pub const TRANSPORT: &[Zone] = &[Zone::MutePrevious, Zone::RecordMenuStopPlay, Zone::VoiceNext];

/// Named key groups Home Assistant can ask for. `None` means "whichever keys the screen would light".
pub fn preset(name: &str) -> Option<&'static [Zone]> {
    use Zone::*;
    match name.trim().to_lowercase().as_str() {
        "all keys" => Some(&Zone::ALL),
        "d-pad" => Some(DPAD),
        "volume" => Some(&[VolumeUpBack, VolumeDown, MutePrevious]),
        "transport" => Some(TRANSPORT),
        "d-pad and volume" => Some(&[DpadUp, DpadLeft, DpadRight, DpadDown, VolumeUpBack, VolumeDown, MutePrevious]),
        "power" => Some(&[ChannelUpPower]),
        _ => None,
    }
}

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

    /// The same pattern of lit and dimmed zones in another colour.
    pub fn recolour(mut self, from: Rgb, to: Rgb) -> Self {
        for c in self.0.iter_mut().filter(|c| !c.is_off()) {
            *c = to.dim(if from.peak() > 0.0 { c.peak() / from.peak() } else { 1.0 });
        }
        self
    }

    /// Fill every zone that is off with `colour`.
    pub fn over(mut self, colour: Rgb) -> Self {
        for c in self.0.iter_mut().filter(|c| c.is_off()) {
            *c = colour;
        }
        self
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// The screen decides the keys and their colour.
    #[default]
    Follow,
    /// Home Assistant's lights decide the colour (and, with the per-key lights, the keys).
    Custom,
    Off,
}

/// What Home Assistant asks of the keys, read from its helper entities.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ha {
    pub mode: Mode,
    /// The "all keys" light, when it is on: the colour for whichever keys are in use.
    pub colour: Option<Rgb>,
    /// A named group of keys to light in place of the screen's own choice.
    pub fixed: Option<&'static [Zone]>,
    /// The per-key lights that are on. In `Custom` mode each overrides its own zone.
    pub zones: [Option<Rgb>; 12],
}

static HA: Mutex<Option<Ha>> = Mutex::new(None);

/// Record Home Assistant's latest wishes; the next frame takes them into account.
pub fn set_ha(ha: Ha) {
    *HA.lock().unwrap_or_else(|e| e.into_inner()) = Some(ha);
}

/// What decides the keys: a snapshot of what the window is showing, plus Home Assistant's wishes.
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
    pub ha: Ha,
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
        ha: HA.lock().unwrap_or_else(|e| e.into_inner()).unwrap_or_default(),
    };
    (state, Rgb(accent.red(), accent.green(), accent.blue()))
}

/// The keys that do something on this screen, in the accent, on an otherwise dark keypad.
fn in_use(s: UiState, accent: Rgb) -> Frame {
    let volume = |frame: Frame| {
        if s.muted {
            // Muted: the mute key is the one to press; volume still works but steps back.
            frame.light(VOLUME, accent.dim(0.25)).light(&[Zone::MutePrevious], accent)
        } else {
            frame.light(VOLUME, accent).light(&[Zone::MutePrevious], accent)
        }
    };
    if let Some(zones) = s.ha.fixed {
        Frame::OFF.light(zones, accent)
    } else if s.page != 0 {
        Frame::OFF.light(DPAD, accent).light(&[Zone::Home, Zone::VolumeUpBack], accent)
    } else if !s.receiver_on {
        // Everything is off; the power key is the only way forward.
        Frame::OFF.light(&[Zone::ChannelUpPower], Rgb::GREEN)
    } else if s.volume_overlay || !s.kodi {
        volume(Frame::OFF)
    } else if s.playing {
        volume(Frame::OFF.light(TRANSPORT, accent))
    } else {
        volume(Frame::OFF.light(DPAD, accent).light(&[Zone::Home, Zone::RecordMenuStopPlay], accent))
    }
}

/// The policy: every key's colour for what is on screen and what Home Assistant asked for.
pub fn frame_for(s: UiState, accent: Rgb) -> Frame {
    if !s.awake || s.ha.mode == Mode::Off {
        return Frame::OFF;
    }
    let custom = s.ha.mode == Mode::Custom;
    let keys = in_use(s, accent);
    let keys = match s.ha.colour.filter(|_| custom) {
        Some(colour) => keys.recolour(accent, colour),
        None => keys,
    };
    let mut frame = keys.over(UNDERLAY);
    if custom {
        // Each per-key light that is on overrides just its own zone.
        for zone in Zone::ALL {
            if let Some(colour) = s.ha.zones[zone.index()] {
                frame = frame.light(&[zone], colour);
            }
        }
    }
    // The power key shows red or green while it is held, whatever else is going on.
    match s.power_hold {
        Some(turning_off) => frame.light(&[Zone::ChannelUpPower], if turning_off { Rgb::RED } else { Rgb::GREEN }),
        None => frame,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCENT: Rgb = Rgb(10, 20, 30);

    fn state() -> UiState {
        UiState {
            awake: true,
            power_hold: None,
            receiver_on: true,
            page: 0,
            kodi: false,
            playing: false,
            volume_overlay: false,
            muted: false,
            ha: Ha::default(),
        }
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
    fn keys_not_in_use_get_the_soft_white_underlay() {
        let f = frame_for(state(), ACCENT);
        assert_eq!(f.get(Zone::VolumeDown), ACCENT);
        assert_eq!(f.get(Zone::DpadUp), UNDERLAY);
        assert_eq!(f.get(Zone::Home), UNDERLAY);
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

    #[test]
    fn home_assistant_can_recolour_or_darken_the_keys() {
        let ha = Ha { mode: Mode::Custom, colour: Some(Rgb(0, 0, 200)), ..Ha::default() };
        let custom = frame_for(UiState { muted: true, ha, ..state() }, ACCENT);
        assert_eq!(custom.get(Zone::MutePrevious), Rgb(0, 0, 200));
        assert_eq!(custom.get(Zone::VolumeDown), Rgb(0, 0, 53), "dimmed keys stay dimmed, in the new colour");
        assert_eq!(custom.get(Zone::Home), UNDERLAY);
        let off = Ha { mode: Mode::Off, ..Ha::default() };
        assert_eq!(frame_for(UiState { ha: off, ..state() }, ACCENT), Frame::OFF);
        let ignored = Ha { mode: Mode::Follow, colour: Some(Rgb(0, 0, 200)), ..Ha::default() };
        assert_eq!(frame_for(UiState { ha: ignored, ..state() }, ACCENT), frame_for(state(), ACCENT));
    }

    #[test]
    fn a_preset_replaces_the_screens_choice_of_keys() {
        let ha = Ha { fixed: preset("D-pad"), ..Ha::default() };
        let f = frame_for(UiState { ha, ..state() }, ACCENT);
        assert_eq!(f.get(Zone::DpadLeft), ACCENT);
        assert_eq!(f.get(Zone::VolumeDown), UNDERLAY);
        assert_eq!(preset("Screen keys"), None);
        assert_eq!(preset("all keys").map(<[Zone]>::len), Some(12));
    }

    #[test]
    fn per_key_lights_override_only_their_own_zone() {
        let mut zones = [None; 12];
        zones[Zone::Home.index()] = Some(Rgb(1, 2, 3));
        zones[Zone::VolumeDown.index()] = Some(Rgb(9, 9, 9));
        let ha = Ha { mode: Mode::Custom, zones, ..Ha::default() };
        let f = frame_for(UiState { ha, ..state() }, ACCENT);
        assert_eq!((f.get(Zone::Home), f.get(Zone::VolumeDown)), (Rgb(1, 2, 3), Rgb(9, 9, 9)));
        assert_eq!(f.get(Zone::MutePrevious), ACCENT, "keys in use keep the screen's colour");
        assert_eq!(f.get(Zone::DpadUp), UNDERLAY);
        let follow = Ha { mode: Mode::Follow, zones, ..Ha::default() };
        assert_eq!(frame_for(UiState { ha: follow, ..state() }, ACCENT), frame_for(state(), ACCENT));
    }
}
