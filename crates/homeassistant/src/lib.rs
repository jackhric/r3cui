//! r3cui's own Home Assistant client. It speaks HA's WebSocket API directly: authenticate with a long-lived token,
//! subscribe to the entities the remote shows, call services, and run Assist voice pipelines (`HaHandle::pipelines`,
//! `HaHandle::run_voice`: audio goes into a `VoiceRun`, what was heard and the answer come out as `AssistEvent`s).
//!
//! Layers, bottom up: `ids` and `wire` (HA's values and frames), `protocol` (its vocabulary), `client` (calls over one
//! `wsclient` connection, matched to replies), `engine` (all the logic, no IO), `state` and `views` (what the UI reads) and
//! `service`/`handle` (running it).

pub mod assist;
mod client;
pub mod config;
pub mod engine;
pub mod error;
mod handle;
pub mod ids;
mod service;
pub mod protocol;
pub mod state;
#[cfg(feature = "testing")]
pub mod testing;
pub mod views;
pub mod wire;

pub use assist::{AssistEvent, EndStage, Pipeline, Pipelines, VoiceError, VoiceRun};
pub use config::HaConfig;
pub use error::{Error, Result};
pub use handle::HaHandle;
pub use ids::EntityId;
pub use service::HaService;
pub use state::{EntityState, HaState, Link};
pub use views::{Feature, Light, MediaPlayer};
