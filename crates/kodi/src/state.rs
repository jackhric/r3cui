//! What the client knows of Kodi, for the UI to show: plain data, published as one value.

use crate::error::Error;
use crate::ids::{AudioStreamIndex, MediaId, PlayerId, SubtitleIndex};
use crate::methods::player::PlayerKind;
use crate::wire::{Art, AudioStream, Speed, SubtitleStream};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default)]
pub struct KodiState {
    pub link: Link,
    /// "Kodi 21.3", once connected.
    pub server: Option<String>,
    /// `None` when nothing plays.
    pub playback: Option<Playback>,
    pub screensaver: bool,
    /// A command's failure, for the UI to show if it likes; it blocks nothing.
    pub last_error: Option<Error>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Link {
    #[default]
    Unconfigured,
    Connecting { attempt: u32 },
    Up,
    Down { error: Error, retry_in: Duration },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Playback {
    pub player: PlayerId,
    pub kind: PlayerKind,
    pub item: NowPlaying,
    pub timeline: Timeline,
    pub audio: Vec<AudioStream>,
    pub audio_current: Option<AudioStreamIndex>,
    pub subtitles: Vec<SubtitleStream>,
    /// `None` when subtitles are off.
    pub subtitle: Option<SubtitleIndex>,
    pub can_seek: bool,
    pub live: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NowPlaying {
    pub id: Option<MediaId>,
    pub title: String,
    /// For an episode: its show, season and number.
    pub show: Option<EpisodeOf>,
    pub year: Option<u16>,
    pub art: Art,
    pub runtime: Option<Duration>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpisodeOf {
    pub show_title: String,
    pub season: u32,
    pub episode: u32,
}

/// Where playback stands, as Kodi said at `measured_at`. The UI asks `position_at` on its own clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timeline {
    pub position: Duration,
    pub total: Duration,
    pub speed: Speed,
    pub measured_at: Instant,
}

impl Timeline {
    pub fn position_at(&self, now: Instant) -> Duration {
        let ran = now.saturating_duration_since(self.measured_at).as_secs_f64() * self.speed.rate();
        let at = (self.position.as_secs_f64() + ran).max(0.0);
        let at = if self.total > Duration::ZERO { at.min(self.total.as_secs_f64()) } else { at };
        Duration::from_secs_f64(at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_runs_at_its_speed() {
        let t0 = Instant::now();
        let t = Timeline { position: Duration::from_secs(100), total: Duration::from_secs(105), speed: Speed::NORMAL, measured_at: t0 };
        assert_eq!(t.position_at(t0 + Duration::from_secs(3)), Duration::from_secs(103));
        assert_eq!(t.position_at(t0 + Duration::from_secs(60)), Duration::from_secs(105), "clamped to the end");
        let paused = Timeline { speed: Speed::PAUSED, ..t };
        assert_eq!(paused.position_at(t0 + Duration::from_secs(60)), Duration::from_secs(100));
    }
}
