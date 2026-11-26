//! The wire messages: `polo` for pairing (port 6467) and `remote` for the remote channel (port 6466), both
//! length-prefixed protobuf. Field numbers are from the Apache-2.0 `.proto` files of androidtvremote2.

pub mod keys;
pub mod polo;
pub mod remote;

pub use keys::{Direction, KeyCode};
