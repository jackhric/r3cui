//! r3cui's own Home Assistant client. It speaks HA's WebSocket API directly: authenticate with a long-lived token,
//! subscribe to the entities the remote shows, and call services.
//!
//! Layers, bottom up: `ids` and `wire` (HA's values and frames), `transport` and `client` (one WebSocket, calls
//! matched to replies), `engine` (all the logic, no IO), `state` and `views` (what the UI reads) and
//! `service`/`handle` (running it).

mod client;
pub mod config;
pub mod engine;
pub mod error;
mod handle;
pub mod ids;
mod service;
pub mod state;
#[cfg(feature = "testing")]
pub mod testing;
mod transport;
pub mod views;
pub mod wire;

pub use config::HaConfig;
pub use error::{Error, Result};
pub use handle::HaHandle;
pub use ids::EntityId;
pub use service::HaService;
pub use state::{EntityState, HaState, Link};
pub use views::{Feature, Light, MediaPlayer};
