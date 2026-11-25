//! r3cui's drivers: the contract the frontend talks to (`api`), the drivers behind it, and the host that runs the
//! ones `modules.json` sets up. See `docs/drivers.md`.
//!
//! A driver module uses only `api` and its own client crate; only `registry` names driver types.

pub mod api;
pub mod config;
pub mod homeassistant;
pub mod host;
pub mod inputs;
pub mod kodi;
pub mod registry;
mod run;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use config::{Binding, Modules, Role, Slot, Store};
pub use host::Host;
pub use inputs::{Input, DEFAULT_INPUT};
pub use registry::Registry;
