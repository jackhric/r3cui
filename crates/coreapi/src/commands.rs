//! Media-player commands (`execute_entity_command`'s `cmd_id` and `params`), as the core's media-player entity docs
//! name them. Each command carries its own parameters, so a command can't go out with the wrong ones.

use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq)]
pub enum MediaPlayerCmd {
    On,
    Off,
    PlayPause,
    Stop,
    Previous,
    Next,
    Record,
    /// To this many seconds in.
    Seek(u32),
    VolumeUp,
    VolumeDown,
    /// To this level.
    Volume(i32),
    MuteToggle,
    Mute,
    Unmute,
    CursorUp,
    CursorDown,
    CursorLeft,
    CursorRight,
    CursorEnter,
    Back,
    Home,
    Menu,
    ChannelUp,
    ChannelDown,
    SelectSource(String),
    SelectSoundMode(String),
}

impl MediaPlayerCmd {
    pub fn id(&self) -> &'static str {
        use MediaPlayerCmd::*;
        match self {
            On => "on",
            Off => "off",
            PlayPause => "play_pause",
            Stop => "stop",
            Previous => "previous",
            Next => "next",
            Record => "record",
            Seek(_) => "seek",
            VolumeUp => "volume_up",
            VolumeDown => "volume_down",
            Volume(_) => "volume",
            MuteToggle => "mute_toggle",
            Mute => "mute",
            Unmute => "unmute",
            CursorUp => "cursor_up",
            CursorDown => "cursor_down",
            CursorLeft => "cursor_left",
            CursorRight => "cursor_right",
            CursorEnter => "cursor_enter",
            Back => "back",
            Home => "home",
            Menu => "menu",
            ChannelUp => "channel_up",
            ChannelDown => "channel_down",
            SelectSource(_) => "select_source",
            SelectSoundMode(_) => "select_sound_mode",
        }
    }

    pub fn params(&self) -> Value {
        use MediaPlayerCmd::*;
        match self {
            Seek(at) => json!({ "media_position": at }),
            Volume(v) => json!({ "volume": v }),
            SelectSource(s) => json!({ "source": s }),
            SelectSoundMode(m) => json!({ "mode": m }),
            _ => Value::Null,
        }
    }

    /// The `execute_entity_command` request for an entity.
    pub fn request(&self, entity_id: &str) -> Value {
        json!({ "entity_id": entity_id, "cmd_id": self.id(), "params": self.params() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_params() {
        assert_eq!(MediaPlayerCmd::CursorRight.request("kodi.x"), json!({ "entity_id": "kodi.x", "cmd_id": "cursor_right", "params": null }));
        assert_eq!(MediaPlayerCmd::Seek(6837).params(), json!({ "media_position": 6837 }));
        assert_eq!(MediaPlayerCmd::SelectSource("KODI".into()).request("avr").get("params"), Some(&json!({ "source": "KODI" })));
    }
}
