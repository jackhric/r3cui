//! Android's `KeyEvent` numbers, the subset worth sending to a TV, and how long a press lasts.

use prost::Enumeration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Enumeration)]
#[repr(i32)]
pub enum KeyCode {
    Home = 3,
    Back = 4,
    Num0 = 7,
    Num1 = 8,
    Num2 = 9,
    Num3 = 10,
    Num4 = 11,
    Num5 = 12,
    Num6 = 13,
    Num7 = 14,
    Num8 = 15,
    Num9 = 16,
    DpadUp = 19,
    DpadDown = 20,
    DpadLeft = 21,
    DpadRight = 22,
    DpadCenter = 23,
    VolumeUp = 24,
    VolumeDown = 25,
    Power = 26,
    Enter = 66,
    Delete = 67,
    Menu = 82,
    Search = 84,
    MediaPlayPause = 85,
    MediaStop = 86,
    MediaNext = 87,
    MediaPrevious = 88,
    MediaRewind = 89,
    MediaFastForward = 90,
    /// KEYCODE_MUTE is the microphone; `VolumeMute` is the speaker.
    MicMute = 91,
    PageUp = 92,
    PageDown = 93,
    MediaPlay = 126,
    MediaPause = 127,
    MediaRecord = 130,
    VolumeMute = 164,
    Info = 165,
    ChannelUp = 166,
    ChannelDown = 167,
    Tv = 170,
    Guide = 172,
    Captions = 175,
    Settings = 176,
    TvPower = 177,
    TvInput = 178,
    ProgRed = 183,
    ProgGreen = 184,
    ProgYellow = 185,
    ProgBlue = 186,
    AppSwitch = 187,
    Sleep = 223,
    Wakeup = 224,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Enumeration)]
#[repr(i32)]
pub enum Direction {
    Unknown = 0,
    StartLong = 1,
    EndLong = 2,
    Short = 3,
}
