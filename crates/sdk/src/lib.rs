//! Write an r3cui integration. An integration is one crate: its own client for the device, a `Spec` saying what it
//! is and what it offers, and a `Driver` that the frontend talks to through ports. It exports an `Integration`, and
//! the app lists it once. See `docs/drivers.md`.

pub mod api;
mod entry;
pub mod run;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use entry::DriverEntry;

use api::{Driver, Spec};
use std::sync::Arc;

/// What an integration crate exports for the app to list: what it is, and how to start one from its setup.
#[derive(Clone, Copy)]
pub struct Integration {
    pub spec: &'static Spec,
    pub start: fn(&DriverEntry) -> Arc<dyn Driver>,
}
