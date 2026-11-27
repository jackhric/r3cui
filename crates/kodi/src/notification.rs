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

/// Kodi's notification names, as they arrive in `method`.
pub mod name {
    pub const PLAYER_ON_PLAY: &str = "Player.OnPlay";
    pub const PLAYER_ON_AV_START: &str = "Player.OnAVStart";
    pub const PLAYER_ON_AV_CHANGE: &str = "Player.OnAVChange";
    pub const PLAYER_ON_PAUSE: &str = "Player.OnPause";
    pub const PLAYER_ON_RESUME: &str = "Player.OnResume";
    pub const PLAYER_ON_SPEED_CHANGED: &str = "Player.OnSpeedChanged";
    pub const PLAYER_ON_SEEK: &str = "Player.OnSeek";
    pub const PLAYER_ON_STOP: &str = "Player.OnStop";
    pub const PLAYER_ON_PROPERTY_CHANGED: &str = "Player.OnPropertyChanged";
    pub const SCREENSAVER_ACTIVATED: &str = "GUI.OnScreensaverActivated";
    pub const DPMS_ACTIVATED: &str = "GUI.OnDPMSActivated";
    pub const SCREENSAVER_DEACTIVATED: &str = "GUI.OnScreensaverDeactivated";
    pub const DPMS_DEACTIVATED: &str = "GUI.OnDPMSDeactivated";
    pub const INPUT_ON_INPUT_REQUESTED: &str = "Input.OnInputRequested";
    pub const INPUT_ON_INPUT_FINISHED: &str = "Input.OnInputFinished";
    pub const VIDEO_LIBRARY_ON_UPDATE: &str = "VideoLibrary.OnUpdate";
    pub const VIDEO_LIBRARY_ON_REMOVE: &str = "VideoLibrary.OnRemove";
    pub const VIDEO_LIBRARY_ON_SCAN_FINISHED: &str = "VideoLibrary.OnScanFinished";
    pub const VIDEO_LIBRARY_ON_CLEAN_FINISHED: &str = "VideoLibrary.OnCleanFinished";
    pub const VIDEO_LIBRARY_ON_REFRESH: &str = "VideoLibrary.OnRefresh";
    pub const SYSTEM_ON_QUIT: &str = "System.OnQuit";
    pub const SYSTEM_ON_RESTART: &str = "System.OnRestart";
    pub const SYSTEM_ON_SLEEP: &str = "System.OnSleep";
    pub const SYSTEM_ON_WAKE: &str = "System.OnWake";
}

impl Notification {
    pub fn parse(method: &str, params: &RawValue) -> Self {
        let data = |m: &str| -> Option<PlayerData> {
            serde_json::from_str::<Params<PlayerData>>(params.get()).map_err(|e| tracing::debug!("{m}: {e}")).ok().map(|p| p.data)
        };
        let player = |m: &str| data(m).map(|d| PlayerEvent { player: d.player.playerid, speed: d.player.speed, item: d.item });
        let parsed = match method {
            name::PLAYER_ON_PLAY => player(method).map(Self::Play),
            name::PLAYER_ON_AV_START => player(method).map(Self::AvStart),
            name::PLAYER_ON_AV_CHANGE => player(method).map(Self::AvChange),
            name::PLAYER_ON_PAUSE => player(method).map(Self::Pause),
            name::PLAYER_ON_RESUME => player(method).map(Self::Resume),
            name::PLAYER_ON_SPEED_CHANGED => player(method).map(Self::SpeedChanged),
            name::PLAYER_ON_SEEK => data(method).map(|d| Self::Seek(SeekEvent { player: d.player.playerid, speed: d.player.speed, time: d.player.time })),
            name::PLAYER_ON_STOP => {
                #[derive(Deserialize)]
                struct Stop {
                    #[serde(default)]
                    end: bool,
                }
                serde_json::from_str::<Params<Stop>>(params.get()).ok().map(|p| Self::Stop { end: p.data.end })
            }
            name::PLAYER_ON_PROPERTY_CHANGED => data(method).map(|d| Self::PropertyChanged { player: d.player.playerid }),
            name::SCREENSAVER_ACTIVATED | name::DPMS_ACTIVATED => Some(Self::Screensaver(true)),
            name::SCREENSAVER_DEACTIVATED | name::DPMS_DEACTIVATED => Some(Self::Screensaver(false)),
            name::INPUT_ON_INPUT_REQUESTED => {
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
            name::INPUT_ON_INPUT_FINISHED => Some(Self::InputFinished),
            name::VIDEO_LIBRARY_ON_UPDATE | name::VIDEO_LIBRARY_ON_REMOVE | name::VIDEO_LIBRARY_ON_SCAN_FINISHED | name::VIDEO_LIBRARY_ON_CLEAN_FINISHED | name::VIDEO_LIBRARY_ON_REFRESH => {
                Some(Self::LibraryChanged)
            }
            name::SYSTEM_ON_QUIT | name::SYSTEM_ON_RESTART | name::SYSTEM_ON_SLEEP => Some(Self::Leaving),
            name::SYSTEM_ON_WAKE => Some(Self::Wake),
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
