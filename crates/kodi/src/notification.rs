//! Kodi's notifications the client acts on, typed after the schema's `notifications` (params `{sender, data}`).
//! Everything else is `Unknown` and ignored.

use crate::ids::PlayerId;
use crate::wire::{Speed, Time};
use serde::Deserialize;
use serde_json::value::RawValue;

#[derive(Debug, Clone, PartialEq)]
pub enum Notification {
    /// `Player.OnPlay` / `OnAVStart`: something started; `OnAVChange`: its streams changed.
    Play(PlayerEvent),
    AvStart(PlayerEvent),
    AvChange(PlayerEvent),
    /// `OnPause`, `OnResume`, `OnSpeedChanged`: the speed in the event is current.
    Pause(PlayerEvent),
    Resume(PlayerEvent),
    SpeedChanged(PlayerEvent),
    Seek(SeekEvent),
    Stop { end: bool },
    PropertyChanged { player: PlayerId },
    Screensaver(bool),
    InputRequested { kind: String, value: String, title: String },
    InputFinished,
    /// The library changed: an entry updated or removed, or a scan or clean finished.
    LibraryChanged,
    /// `System.OnQuit`, `OnRestart`, `OnSleep`: Kodi is going away.
    Leaving,
    Wake,
    Unknown { method: String },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PlayerEvent {
    pub player: PlayerId,
    pub speed: Speed,
    pub item: NotifyItem,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SeekEvent {
    pub player: PlayerId,
    pub speed: Speed,
    /// Where it seeked to.
    pub time: Option<Time>,
}

/// `Notifications.Item`: a library item by id, or a description of one outside the library.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct NotifyItem {
    /// `Notifications.Item.Type`: "movie", "episode", "song", …
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub id: Option<u32>,
    #[serde(default)]
    pub title: String,
}

#[derive(Deserialize)]
struct Params<T> {
    data: T,
}

#[derive(Deserialize)]
struct PlayerData {
    player: PlayerSpeed,
    #[serde(default)]
    item: NotifyItem,
}

#[derive(Deserialize)]
struct PlayerSpeed {
    playerid: PlayerId,
    #[serde(default)]
    speed: Speed,
    #[serde(default)]
    time: Option<Time>,
}

impl Notification {
    pub fn parse(method: &str, params: &RawValue) -> Self {
        let data = |m: &str| -> Option<PlayerData> {
            serde_json::from_str::<Params<PlayerData>>(params.get()).map_err(|e| tracing::debug!("{m}: {e}")).ok().map(|p| p.data)
        };
        let player = |m: &str| data(m).map(|d| PlayerEvent { player: d.player.playerid, speed: d.player.speed, item: d.item });
        let parsed = match method {
            "Player.OnPlay" => player(method).map(Self::Play),
            "Player.OnAVStart" => player(method).map(Self::AvStart),
            "Player.OnAVChange" => player(method).map(Self::AvChange),
            "Player.OnPause" => player(method).map(Self::Pause),
            "Player.OnResume" => player(method).map(Self::Resume),
            "Player.OnSpeedChanged" => player(method).map(Self::SpeedChanged),
            "Player.OnSeek" => data(method).map(|d| Self::Seek(SeekEvent { player: d.player.playerid, speed: d.player.speed, time: d.player.time })),
            "Player.OnStop" => {
                #[derive(Deserialize)]
                struct Stop {
                    #[serde(default)]
                    end: bool,
                }
                serde_json::from_str::<Params<Stop>>(params.get()).ok().map(|p| Self::Stop { end: p.data.end })
            }
            "Player.OnPropertyChanged" => data(method).map(|d| Self::PropertyChanged { player: d.player.playerid }),
            "GUI.OnScreensaverActivated" | "GUI.OnDPMSActivated" => Some(Self::Screensaver(true)),
            "GUI.OnScreensaverDeactivated" | "GUI.OnDPMSDeactivated" => Some(Self::Screensaver(false)),
            "Input.OnInputRequested" => {
                #[derive(Deserialize)]
                struct Asked {
                    #[serde(rename = "type")]
                    kind: String,
                    #[serde(default)]
                    value: String,
                    #[serde(default)]
                    title: String,
                }
                serde_json::from_str::<Params<Asked>>(params.get()).ok().map(|p| Self::InputRequested { kind: p.data.kind, value: p.data.value, title: p.data.title })
            }
            "Input.OnInputFinished" => Some(Self::InputFinished),
            "VideoLibrary.OnUpdate" | "VideoLibrary.OnRemove" | "VideoLibrary.OnScanFinished" | "VideoLibrary.OnCleanFinished" | "VideoLibrary.OnRefresh" => {
                Some(Self::LibraryChanged)
            }
            "System.OnQuit" | "System.OnRestart" | "System.OnSleep" => Some(Self::Leaving),
            "System.OnWake" => Some(Self::Wake),
            _ => None,
        };
        parsed.unwrap_or_else(|| Self::Unknown { method: method.to_string() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn parse(method: &str, params: &str) -> Notification {
        Notification::parse(method, &RawValue::from_string(params.into()).unwrap())
    }

    #[test]
    fn player_events() {
        let n = parse("Player.OnPause", r#"{"data":{"item":{"id":39,"type":"movie"},"player":{"playerid":1,"speed":0}},"sender":"xbmc"}"#);
        assert_eq!(n, Notification::Pause(PlayerEvent { player: PlayerId(1), speed: Speed::PAUSED, item: NotifyItem { kind: "movie".into(), id: Some(39), title: String::new() } }));
        let n = parse("Player.OnSeek", r#"{"data":{"item":{"id":39,"type":"movie"},"player":{"playerid":1,"seekoffset":{"hours":0,"milliseconds":0,"minutes":0,"seconds":-3},"speed":1,"time":{"hours":1,"milliseconds":250,"minutes":53,"seconds":57}}},"sender":"xbmc"}"#);
        let Notification::Seek(s) = n else { panic!("{n:?}") };
        assert_eq!(s.time, Some(Time(Duration::from_millis(6_837_250))));
        assert_eq!(parse("Player.OnStop", r#"{"data":{"end":true,"item":{"id":39,"type":"movie"}},"sender":"xbmc"}"#), Notification::Stop { end: true });
    }

    #[test]
    fn others() {
        assert_eq!(parse("GUI.OnScreensaverActivated", r#"{"data":null,"sender":"xbmc"}"#), Notification::Screensaver(true));
        assert_eq!(parse("System.OnSleep", r#"{"data":null,"sender":"xbmc"}"#), Notification::Leaving);
        assert_eq!(parse("AudioLibrary.OnUpdate", "{}"), Notification::Unknown { method: "AudioLibrary.OnUpdate".into() });
    }
}
