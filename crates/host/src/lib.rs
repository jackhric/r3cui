//! r3cui's side of its drivers: `modules.json` (which drivers are set up, what global control points at, the input
//! list), the host that runs them, and the targets they last offered. The drivers themselves are integrations, each
//! its own crate on `r3cui-sdk`, listed by the app; nothing here names one. See `docs/drivers.md`.

pub mod config;
pub mod host;
pub mod inputs;
pub mod known;
pub mod registry;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use config::{Binding, Modules, Role, Slot, Store};
pub use host::Host;
pub use inputs::{Input, DEFAULT_INPUT};
pub use known::KnownTargets;
pub use registry::Registry;
