//! r3cui's own Kodi client. It talks to Kodi's JSON-RPC directly, so the remote gets exactly what it needs rather
//! than what a core integration passes on. The design, and where each feature lives, is in docs/kodi-client.md.
//!
//! Layers, bottom up: `ids` and `wire` (Kodi's values), `rpc` (the envelope and the `Method` trait), `methods` (one
//! typed struct per call), `notification` (Kodi's events); then `client` (calls over one `wsclient` connection, matched
//! to replies), `engine` (all the logic, no IO), `state` (what the UI reads) and `service`/`handle` (running it).
//!
//! With the default `driver` feature it is also the r3cui integration: `driver` holds its spec and its driver, on
//! `r3cui-sdk`, and `driver::INTEGRATION` is what the app lists. See docs/drivers.md.

#[cfg(feature = "driver")]
pub mod driver;
pub mod art;
pub mod browse;
mod client;
pub mod config;
pub mod engine;
pub mod error;
mod handle;
pub mod ids;
pub mod labels;
pub mod methods;
pub mod notification;
pub mod rpc;
mod service;
pub mod state;
#[cfg(feature = "testing")]
pub mod testing;
pub mod wire;

pub use config::{Credentials, KodiConfig};
pub use error::{Error, Result};
pub use handle::{KodiHandle, Media, Nav, StartAt, SubtitleChoice};
pub use service::KodiService;
pub use state::{KodiState, Link, NowPlaying, Playback, Timeline};
