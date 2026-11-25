//! The receiver, a media player in Home Assistant: its volume, mute, input and power, shown as asked here until it
//! reports the change itself (it can take seconds, and a minute to come on), and its volume scale.

use crate::app::entity::{list, number, text};
use crate::app::optimistic::Optimistic;
use crate::app::settings;
use coreapi::{Attr, Entity, MediaPlayerCmd, MediaState};
use std::time::Duration;

/// How long the slider's level wins outright, and the longest it is shown while the receiver hasn't reported it.
const SLIDER_HOLD: Duration = Duration::from_millis(1500);
const SLIDER_SETTLE: Duration = Duration::from_secs(6);
/// How long an input picked here is shown before the receiver's own report wins again.
const SWITCH_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a power change made here is believed before the receiver's own report wins again.
const POWER_TIMEOUT: Duration = Duration::from_secs(120);

pub(crate) struct Receiver {
    /// The media player the volume keys drive.
    pub(crate) id: Option<String>,
    /// The level the slider last asked for: steps taken before the receiver reports it build on this.
    volume: Optimistic<i32>,
    source: Optimistic<String>,
    power: Optimistic<bool>,
    /// So a second double tap undoes the first even before the receiver reports.
    mute: Optimistic<bool>,
}

/// What the screen shows of the receiver.
pub(crate) struct View {
    pub(crate) volume: i32,
    pub(crate) volume_max: i32,
    pub(crate) muted: bool,
    pub(crate) source: String,
    pub(crate) sources: Vec<String>,
    /// An input picked here that the receiver hasn't confirmed yet.
    pub(crate) switching: bool,
    pub(crate) on: bool,
}

impl Receiver {
    pub(crate) fn new(id: Option<String>) -> Self {
        Self {
            id,
            volume: Optimistic::new(SLIDER_HOLD, SLIDER_SETTLE),
            source: Optimistic::new(Duration::ZERO, SWITCH_TIMEOUT),
            power: Optimistic::new(Duration::ZERO, POWER_TIMEOUT),
            mute: Optimistic::new(Duration::ZERO, SLIDER_SETTLE),
        }
    }

    /// `receiver` is its entity; volume and mute come from `volume_from`, which is Kodi's own when there is no
    /// receiver.
    pub(crate) fn view(&self, receiver: Option<&Entity>, volume_from: Option<&Entity>, entities: &[Entity]) -> View {
        let reported = text(receiver, Attr::Source).to_string();
        View {
            volume: self.volume.resolve(number(volume_from, Attr::Volume).round() as i32),
            volume_max: volume_max(entities),
            muted: self.mute.resolve(volume_from.and_then(|e| e.flag(Attr::Muted)).unwrap_or(false)),
            switching: self.source.pending(&reported),
            source: self.source.resolve(reported),
            sources: list(receiver, Attr::SourceList).into_iter().filter(|s| !settings::get().hides(s)).collect(),
            // The receiver is the source of truth: anything short of on (standby, unreachable, not yet known) is off.
            on: self.power.resolve(receiver.is_none_or(|r| !matches!(r.state(), MediaState::Off | MediaState::Standby) && !r.state().is_gone())),
        }
    }

    pub(crate) fn select_source(&mut self, name: String) -> MediaPlayerCmd {
        self.source.ask(name.clone());
        MediaPlayerCmd::SelectSource(name)
    }

    /// Switch on if off, off if on.
    pub(crate) fn toggle_power(&mut self, on: bool) -> MediaPlayerCmd {
        self.power.ask(!on);
        if on { MediaPlayerCmd::Off } else { MediaPlayerCmd::On }
    }

    /// A slider step to `level`.
    pub(crate) fn slide_to(&mut self, level: i32) {
        self.volume.ask(level);
    }

    /// Not every player has a toggle (the Denon in Home Assistant only has mute and unmute), and the receiver
    /// reports late, so the toggle follows what this remote last asked for.
    pub(crate) fn toggle_mute(&mut self, muted: bool, entity: &Entity) -> MediaPlayerCmd {
        self.mute.ask(!muted);
        if entity.has_feature("mute_toggle") {
            MediaPlayerCmd::MuteToggle
        } else if muted {
            MediaPlayerCmd::Unmute
        } else {
            MediaPlayerCmd::Mute
        }
    }
}

/// The receiver's volume limit, from the Home Assistant sensor that reads it; full scale without one.
fn volume_max(entities: &[Entity]) -> i32 {
    let suffix = &settings::get().volume_limit_sensor;
    entities
        .iter()
        .find(|e| !suffix.is_empty() && e.entity_id.ends_with(suffix.as_str()))
        .and_then(|e| e.attributes.get(Attr::Value.name()))
        .and_then(|v| v.as_f64().or_else(|| v.as_str()?.trim().parse().ok()))
        .map(|v| (v.round() as i32).clamp(1, MAX_VOLUME))
        .unwrap_or(MAX_VOLUME)
}


/// The Denon tops out at 98 (+18 dB).
pub(crate) const MAX_VOLUME: i32 = 98;
/// The strip is 100 steps from silence to the receiver's limit; never less than one level a step.
pub(crate) fn slide(from: i32, steps: i32, max: i32) -> i32 {
    let by = (steps as f64 * max as f64 / 100.0).round() as i32;
    let by = if by == 0 { steps.signum() } else { by };
    (from + by).clamp(0, max)
}
pub(crate) fn volume_percent(volume: i32, max: i32) -> i32 {
    ((volume.max(0) as f64 / max.max(1) as f64) * 100.0).round().min(100.0) as i32
}
/// Denon reports volume as dB + 80, so the receiver's own dB reading is the level minus 80.
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
