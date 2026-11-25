//! Values that appear in many of Kodi's messages, as the schema defines them, with their Rust meaning.

use crate::ids::{AudioStreamIndex, SubtitleIndex};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::time::Duration;

/// `Global.Time` / `Player.Position.Time`: hours, minutes, seconds and milliseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Time(pub Duration);

#[derive(Serialize, Deserialize)]
struct TimeParts {
    #[serde(default)]
    hours: u64,
    #[serde(default)]
    minutes: u64,
    #[serde(default)]
    seconds: u64,
    #[serde(default)]
    milliseconds: u64,
}

impl Serialize for Time {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let ms = self.0.as_millis() as u64;
        TimeParts { hours: ms / 3_600_000, minutes: ms / 60_000 % 60, seconds: ms / 1000 % 60, milliseconds: ms % 1000 }.serialize(s)
    }
}

impl<'de> Deserialize<'de> for Time {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let t = TimeParts::deserialize(d)?;
        Ok(Self(Duration::from_millis(((t.hours * 60 + t.minutes) * 60 + t.seconds) * 1000 + t.milliseconds)))
    }
}

/// `Player.Speed`'s integer: 0 paused, 1 playing, powers of two up to ±32 for fast-forward and rewind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Speed(pub i32);

impl Speed {
    pub const PAUSED: Speed = Speed(0);
    pub const NORMAL: Speed = Speed(1);

    pub fn is_paused(self) -> bool {
        self.0 == 0
    }

    /// How many seconds of the item pass per second.
    pub fn rate(self) -> f64 {
        self.0 as f64
    }
}

/// `Player.Audio.Stream`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct AudioStream {
    pub index: AudioStreamIndex,
    #[serde(default)]
    pub name: String,
    /// ISO 639-2, such as "eng"; may be empty.
    #[serde(default)]
    pub language: String,
    /// Kodi's codec id, such as "dtshd_ma", "truehd", "eac3".
    #[serde(default)]
    pub codec: String,
    #[serde(default)]
    pub channels: u8,
    #[serde(default)]
    pub bitrate: u32,
    #[serde(default, rename = "isdefault")]
    pub default: bool,
    #[serde(default, rename = "isoriginal")]
    pub original: bool,
    #[serde(default, rename = "isimpaired")]
    pub impaired: bool,
}

/// `Player.Subtitle`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct SubtitleStream {
    pub index: SubtitleIndex,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub language: String,
    #[serde(default, rename = "isforced")]
    pub forced: bool,
    #[serde(default, rename = "isimpaired")]
    pub impaired: bool,
    #[serde(default, rename = "isdefault")]
    pub default: bool,
}

/// One of Kodi's art references, such as `image://https%3a%2f%2f…%2f/`: what `/image/` serves.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ArtRef(pub String);

/// `Media.Artwork`: the kinds the UI shows. Shows' items carry the show's art under `tvshow.*`.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Art {
    #[serde(default)]
    pub poster: Option<ArtRef>,
    #[serde(default)]
    pub fanart: Option<ArtRef>,
    #[serde(default)]
    pub thumb: Option<ArtRef>,
    #[serde(default, rename = "tvshow.poster")]
    pub show_poster: Option<ArtRef>,
    #[serde(default, rename = "tvshow.fanart")]
    pub show_fanart: Option<ArtRef>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn time_round_trips_through_kodis_parts() {
        let t = Time(Duration::from_millis(6_837_250));
        let v = serde_json::to_value(t).unwrap();
        assert_eq!(v, json!({ "hours": 1, "minutes": 53, "seconds": 57, "milliseconds": 250 }));
        assert_eq!(serde_json::from_value::<Time>(v).unwrap(), t);
        assert_eq!(serde_json::from_value::<Time>(json!({ "seconds": 5 })).unwrap(), Time(Duration::from_secs(5)));
    }
}
