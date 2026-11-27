//! The bridge link: a WebSocket on localhost over which the bridge (the integration that receives the core's audio)
//! hands a session's audio to r3cui and r3cui reports the outcome. The frames are in `docs/voice.md`.

mod client;
mod frames;
mod server;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use client::{connect_loop, connect_to, Link, LinkEvent, LinkHandle};
pub(crate) use server::serve;
pub use frames::{BridgeMsg, ErrorCode, SampleFormat, SessionId, UiMsg};

/// The bridge as the core knows it: `driver_id` in its `driver.json`, and its voice assistant entity.
pub const BRIDGE_DRIVER_ID: &str = "r3cui_bridge";
pub const VOICE_ENTITY_ID: &str = "voice";

pub const PORT: u16 = 10790;
pub const HOST: &str = "127.0.0.1";
