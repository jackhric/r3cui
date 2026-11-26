//! Remote control of r3cui: one WebSocket at `ws://<remote>:PORT/control`. The remote first sends a `Challenge`; the
//! client answers with `Hello`, proving it holds the access key without sending it; then each `Request` gets a `Reply`
//! with the same `id`. A screenshot's PNG follows its reply as one binary frame.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const PORT: u16 = 10780;
pub const PATH: &str = "/control";

/// The hard keys a client may press. Power is left out on purpose: switching the remote off stays a physical act.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HardKey {
    Back,
    Home,
    Menu,
    Up,
    Down,
    Left,
    Right,
    Ok,
    VolumeUp,
    VolumeDown,
    Mute,
    ChannelUp,
    ChannelDown,
    Prev,
    Play,
    Next,
    Stop,
    Record,
    Voice,
}

impl HardKey {
    pub const ALL: [HardKey; 19] = [
        Self::Back, Self::Home, Self::Menu, Self::Up, Self::Down, Self::Left, Self::Right, Self::Ok, Self::VolumeUp, Self::VolumeDown, Self::Mute,
        Self::ChannelUp, Self::ChannelDown, Self::Prev, Self::Play, Self::Next, Self::Stop, Self::Record, Self::Voice,
    ];

    /// The name on the wire and on a command line.
    pub fn name(self) -> String {
        serde_json::to_value(self).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.name() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyAction {
    Press,
    Release,
    /// Press and release, as a quick tap.
    Tap,
    /// Press, hold this long, release; timed on the remote so the network cannot shorten it.
    Hold { ms: u32 },
}

/// Screen coordinates in the 480×800 portrait layout.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Key { key: HardKey, action: KeyAction },
    Tap { at: Point },
    Swipe { from: Point, to: Point, ms: u32 },
    /// The touch slider: positive steps up, negative down, as a finger moving along it.
    Slide { steps: i32 },
    /// Go straight to a page by its name (the `--page` names).
    Page { name: String },
    State,
    Screenshot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope<T> {
    pub id: u64,
    #[serde(flatten)]
    pub body: T,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reply {
    Ok,
    Error { message: String },
    State(State),
    /// A PNG of `bytes` follows as a binary frame.
    Screenshot { width: u32, height: u32, bytes: u64, capture_ms: f32, encode_ms: f32 },
}

/// What the remote shows, as far as a client needs to know where it is.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub page: String,
    pub focus: i32,
    pub input: String,
    pub voice: String,
    pub awake: bool,
    pub battery: i32,
    pub charging: bool,
}

/// Sent by the remote first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Challenge {
    Challenge { nonce: String },
}

/// The client's answer to the challenge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Hello {
    Hello { proof: String },
}

/// HMAC-SHA256 of `nonce` under `key`, in hex: the proof that a client holds the key.
pub fn proof(key: &str, nonce: &str) -> String {
    const BLOCK: usize = 64;
    let mut k = key.as_bytes().to_vec();
    if k.len() > BLOCK {
        k = Sha256::digest(&k).to_vec();
    }
    k.resize(BLOCK, 0);
    let pad = |byte: u8| k.iter().map(|b| b ^ byte).collect::<Vec<u8>>();
    let inner = Sha256::new().chain_update(pad(0x36)).chain_update(nonce.as_bytes()).finalize();
    let outer = Sha256::new().chain_update(pad(0x5c)).chain_update(inner).finalize();
    outer.iter().map(|b| format!("{b:02x}")).collect()
}

/// Comparing proofs in constant time, so timing reveals nothing about the right one.
pub fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proof_is_hmac_sha256() {
        // RFC 4231, test case 2.
        assert_eq!(proof("Jefe", "what do ya want for nothing?"), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        assert!(same(&proof("k", "n"), &proof("k", "n")) && !same(&proof("k", "n"), &proof("j", "n")));
    }

    #[test]
    fn messages_have_their_wire_shape() {
        let r = Envelope { id: 3, body: Request::Key { key: HardKey::VolumeUp, action: KeyAction::Hold { ms: 800 } } };
        assert_eq!(serde_json::to_string(&r).unwrap(), r#"{"id":3,"type":"key","key":"volume_up","action":{"hold":{"ms":800}}}"#);
        let r = Envelope { id: 4, body: Request::Tap { at: Point { x: 240.0, y: 400.0 } } };
        assert_eq!(serde_json::from_str::<Envelope<Request>>(r#"{"id":4,"type":"tap","at":{"x":240,"y":400}}"#).unwrap(), r);
        assert_eq!(HardKey::from_name("voice"), Some(HardKey::Voice));
        assert!(HardKey::ALL.iter().all(|k| HardKey::from_name(&k.name()) == Some(*k)));
    }
}
