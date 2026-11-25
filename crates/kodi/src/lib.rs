//! r3cui's own Kodi client. It talks to Kodi's JSON-RPC directly, so the remote gets exactly what it needs rather
//! than what a core integration passes on. The design, and where each feature lives, is in docs/kodi-client.md.
//!
//! Layers, bottom up: `ids` and `wire` (Kodi's values), `rpc` (the envelope and the `Method` trait), `methods` (one
//! typed struct per call), `notification` (Kodi's events).

pub mod error;
pub mod ids;
pub mod methods;
pub mod notification;
pub mod rpc;
pub mod wire;

pub use error::{Error, Result};
