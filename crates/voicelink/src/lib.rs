//! The voice link: a WebSocket on localhost over which `voice-relay` (the integration that receives the core's audio)
//! hands a session's audio to r3cui and r3cui reports the outcome. The frames are in `docs/voice.md`.

mod client;
mod frames;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use client::{connect_loop, connect_to, Link, LinkEvent, LinkHandle};
pub use frames::{ErrorCode, RelayMsg, SampleFormat, SessionId, UiMsg};

/// The relay as the core knows it: `driver_id` in its `driver.json`, and its one entity, the voice assistant.
pub const RELAY_DRIVER_ID: &str = "r3cui_voice";
pub const RELAY_ENTITY_ID: &str = "voice";

pub const PORT: u16 = 10790;
pub const HOST: &str = "127.0.0.1";
