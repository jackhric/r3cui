//! Global control as the screen shows it: the volume and input the host reports, with what was asked here shown
//! until the device confirms it (it can take seconds, and a minute to come on), and the volume scale.

use crate::app::optimistic::Optimistic;
use r3cui_sdk::api::{PowerIntent, SourceIntent, Sources, Volume, VolumeIntent};
use host::Host;
use std::time::Duration;

/// How long the slider's level wins outright, and the longest it is shown while the device hasn't reported it.
const SLIDER_HOLD: Duration = Duration::from_millis(1500);
const SLIDER_SETTLE: Duration = Duration::from_secs(6);
/// How long an input picked here is shown before the device's own report wins again.
const SWITCH_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a power change made here is believed before the device's own report wins again.
const POWER_TIMEOUT: Duration = Duration::from_secs(120);

pub(crate) struct Global {
    volume: Optimistic<i32>,
    input: Optimistic<String>,
    power: Optimistic<bool>,
    /// So a second double tap undoes the first even before the device reports.
    mute: Optimistic<bool>,
}

/// What the screen shows of global control.
#[derive(Debug, Clone, Default)]
pub(crate) struct View {
    pub(crate) volume: i32,
    pub(crate) volume_max: i32,
    pub(crate) muted: bool,
    /// The input on now, by id.
    pub(crate) input: String,
    /// An input picked here that the device hasn't confirmed yet.
    pub(crate) switching: bool,
    pub(crate) on: bool,
}

impl Default for Global {
    fn default() -> Self {
        Self {
            volume: Optimistic::new(SLIDER_HOLD, SLIDER_SETTLE),
            input: Optimistic::new(Duration::ZERO, SWITCH_TIMEOUT),
            power: Optimistic::new(Duration::ZERO, POWER_TIMEOUT),
            mute: Optimistic::new(Duration::ZERO, SLIDER_SETTLE),
        }
    }
}

impl Global {
    pub(crate) fn view(&self, host: &Host) -> View {
        let v = host.volume().unwrap_or(Volume { level: 0, max: MAX_VOLUME, muted: false });
        let s: Option<Sources> = host.sources();
        let reported = s.as_ref().map(|s| s.current.clone()).unwrap_or_default();
        View {
            volume: self.volume.resolve(v.level),
            volume_max: v.max,
            muted: self.mute.resolve(v.muted),
            switching: self.input.pending(&reported),
            input: self.input.resolve(reported),
            // The device is the source of truth: anything short of on is off. With no power binding there is nothing to
            // switch off.
            on: self.power.resolve(host.power().is_none_or(|p| p.on)),
        }
    }

    pub(crate) fn select_input(&mut self, host: &Host, id: String) {
        self.input.ask(id.clone());
        host.send_source(SourceIntent::Select(id));
    }

    /// Switch on if off, off if on.
    pub(crate) fn toggle_power(&mut self, host: &Host, on: bool) {
        self.power.ask(!on);
        host.send_power(PowerIntent::Set(!on));
    }

    /// A slider step to `level`; the slider's pacing decides when it is sent.
    pub(crate) fn slide_to(&mut self, level: i32) {
        self.volume.ask(level);
    }

    /// Follows what this remote last asked for, since the device reports late.
    pub(crate) fn toggle_mute(&mut self, host: &Host, muted: bool) {
        self.mute.ask(!muted);
        host.send_volume(VolumeIntent::Mute(!muted));
    }
}

/// Full scale when the volume has no limit of its own.
pub(crate) const MAX_VOLUME: i32 = 100;
/// The strip is 100 steps from silence to the volume limit; never less than one level a step.
pub(crate) fn slide(from: i32, steps: i32, max: i32) -> i32 {
    let by = (steps as f64 * max as f64 / 100.0).round() as i32;
    let by = if by == 0 { steps.signum() } else { by };
    (from + by).clamp(0, max)
}
pub(crate) fn volume_percent(volume: i32, max: i32) -> i32 {
    ((volume.max(0) as f64 / max.max(1) as f64) * 100.0).round().min(100.0) as i32
}
/// Receivers that report volume as dB + 80 (Denon, Marantz): their own dB reading is the level minus 80.
pub(crate) fn volume_db(volume: i32) -> String {
    match volume - 80 {
        db if db < 0 => format!("−{} dB", -db),
        0 => "0 dB".to_string(),
        db => format!("+{db} dB"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_scale() {
        assert_eq!(volume_db(62), "−18 dB");
        assert_eq!(volume_db(80), "0 dB");
        assert_eq!(volume_percent(72, 72), 100);
        assert_eq!(volume_percent(36, 72), 50);
        assert_eq!(volume_percent(80, 72), 100);
        assert_eq!(slide(0, 100, 72), 72);
        assert_eq!(slide(36, 1, 72), 37);
        assert_eq!(slide(36, -50, 72), 0);
    }
}
