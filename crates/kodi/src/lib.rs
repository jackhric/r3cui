//! r3cui's own Kodi client. It talks to Kodi's JSON-RPC directly, so the remote gets exactly what it needs rather
//! than what a core integration passes on. The design, and where each feature lives, is in docs/kodi-client.md.
//!
//! Layers, bottom up: `ids` and `wire` (Kodi's values), `rpc` (the envelope and the `Method` trait), `methods` (one
//! typed struct per call), `notification` (Kodi's events); then `transport` and `client` (one WebSocket, calls matched
//! to replies), `engine` (all the logic, no IO), `state` (what the UI reads) and `service`/`handle` (running it).

pub mod art;
mod client;
pub mod config;
pub mod engine;
pub mod error;
mod handle;
pub mod ids;
pub mod methods;
pub mod notification;
pub mod rpc;
mod service;
pub mod state;
mod transport;
pub mod wire;

pub use config::{Credentials, KodiConfig};
pub use error::{Error, Result};
pub use handle::KodiHandle;
pub use service::KodiService;
pub use state::{KodiState, Link, NowPlaying, Playback, Timeline};
