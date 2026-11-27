//! `Player.*`: what's playing, its timeline and streams, and controlling it.

use crate::ids::{AudioStreamIndex, EpisodeId, MovieId, PlayerId, SubtitleIndex};
use crate::rpc::Method;
use crate::wire::{Art, AudioStream, Speed, SubtitleStream, Time};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize)]
pub struct GetActivePlayers {}
impl Method for GetActivePlayers {
    const NAME: &'static str = "Player.GetActivePlayers";
    type Output = Vec<ActivePlayer>;
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ActivePlayer {
    pub playerid: PlayerId,
    #[serde(rename = "type")]
    pub kind: PlayerKind,
}

/// `Player.Type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlayerKind {
    Video,
    Audio,
    Picture,
}

/// The subset of `List.Fields.All` asked of what's playing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemField {
    Title,
    ShowTitle,
    Season,
    Episode,
    Year,
    Runtime,
    Art,
    File,
}

#[derive(Debug, Clone, Serialize)]
pub struct GetItem {
    pub playerid: PlayerId,
    pub properties: &'static [ItemField],
}
impl Method for GetItem {
    const NAME: &'static str = "Player.GetItem";
    type Output = GetItemResult;
}

#[derive(Debug, Clone, Deserialize)]
pub struct GetItemResult {
    pub item: PlayingItem,
}

/// `List.Item.All`, the fields asked for.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct PlayingItem {
    /// The library id, with `type` saying of what; absent for files outside the library.
    #[serde(default)]
    pub id: Option<u32>,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, rename = "showtitle")]
    pub show_title: String,
    #[serde(default)]
    pub season: i32,
    #[serde(default)]
    pub episode: i32,
    #[serde(default)]
    pub year: i32,
    /// Seconds.
    #[serde(default)]
    pub runtime: u32,
    #[serde(default)]
    pub art: Art,
    #[serde(default)]
    pub file: String,
}

/// The subset of `Player.Property.Name` the client reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlayerProperty {
    Speed,
    Time,
    TotalTime,
    CanSeek,
    Live,
    AudioStreams,
    CurrentAudioStream,
    Subtitles,
    CurrentSubtitle,
    SubtitleEnabled,
}

#[derive(Debug, Clone, Serialize)]
pub struct GetProperties {
    pub playerid: PlayerId,
    pub properties: &'static [PlayerProperty],
}
impl Method for GetProperties {
    const NAME: &'static str = "Player.GetProperties";
    type Output = PlayerProps;
}

/// `Player.Property.Value`: only what was asked comes back.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct PlayerProps {
    pub speed: Option<Speed>,
    pub time: Option<Time>,
    pub totaltime: Option<Time>,
    pub canseek: Option<bool>,
    pub live: Option<bool>,
    pub audiostreams: Option<Vec<AudioStream>>,
    /// Kodi answers an empty object when there is none.
    #[serde(default, deserialize_with = "maybe_stream")]
    pub currentaudiostream: Option<AudioStream>,
    pub subtitles: Option<Vec<SubtitleStream>>,
    #[serde(default, deserialize_with = "maybe_stream")]
    pub currentsubtitle: Option<SubtitleStream>,
    pub subtitleenabled: Option<bool>,
}

fn maybe_stream<'de, D: serde::Deserializer<'de>, T: serde::de::DeserializeOwned>(d: D) -> Result<Option<T>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(v).ok())
}

/// `Global.Toggle`: switch, or set either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    Toggle,
    On,
    Off,
}

impl Serialize for Toggle {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Toggle => s.serialize_str("toggle"),
            Self::On => s.serialize_bool(true),
            Self::Off => s.serialize_bool(false),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PlayPause {
    pub playerid: PlayerId,
    pub play: Toggle,
}
impl Method for PlayPause {
    const NAME: &'static str = "Player.PlayPause";
    type Output = SpeedResult;
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct SpeedResult {
    pub speed: Speed,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stop {
    pub playerid: PlayerId,
}
impl Method for Stop {
    const NAME: &'static str = "Player.Stop";
    type Output = serde::de::IgnoredAny;
}

#[derive(Debug, Clone, Serialize)]
pub struct Seek {
    pub playerid: PlayerId,
    pub value: SeekTo,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(untagged)]
pub enum SeekTo {
    Time { time: Time },
    /// Relative, in seconds.
    Seconds { seconds: i32 },
}
impl Method for Seek {
    const NAME: &'static str = "Player.Seek";
    type Output = SeekResult;
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct SeekResult {
    pub time: Time,
    pub totaltime: Time,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetAudioStream {
    pub playerid: PlayerId,
    pub stream: AudioStreamIndex,
}
impl Method for SetAudioStream {
    const NAME: &'static str = "Player.SetAudioStream";
    type Output = serde::de::IgnoredAny;
}

#[derive(Debug, Clone, Serialize)]
pub struct SetSubtitle {
    pub playerid: PlayerId,
    pub subtitle: SubtitleTarget,
    pub enable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum SubtitleTarget {
    Index(SubtitleIndex),
    Word(SubtitleWord),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SubtitleWord {
    Off,
    On,
}
impl Method for SetSubtitle {
    const NAME: &'static str = "Player.SetSubtitle";
    type Output = serde::de::IgnoredAny;
}

/// Open a library item, from its resume point or the start.
#[derive(Debug, Clone, Serialize)]
pub struct Open {
    pub item: OpenItem,
    pub options: OpenOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum OpenItem {
    Movie { movieid: MovieId },
    Episode { episodeid: EpisodeId },
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenOptions {
    pub resume: bool,
}
impl Method for Open {
    const NAME: &'static str = "Player.Open";
    type Output = serde::de::IgnoredAny;
}
