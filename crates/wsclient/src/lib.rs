//! The WebSocket plumbing shared by r3cui's driver crates: a connection that is pinged while idle and given up on
//! when silent (`connect`), the backoff between attempts (`Backoff`), a reconnect loop that reports each attempt and
//! outcome (`reconnect`), and the table matching replies to the calls waiting for them (`Pending`).
//!
//! A driver crate keeps its own protocol: what the frames mean, how to authenticate, what state to publish. With the
//! `testing` feature, `testing` has a scaffold for the fake servers its tests run against.

mod backoff;
mod connection;
mod pending;
mod reconnect;
#[cfg(feature = "tls")]
pub mod tls;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use backoff::{delay_for, Backoff, STABLE_AFTER};
#[cfg(feature = "tls")]
pub use connection::connect_trusting;
pub use connection::{connect, connect_with, Closed, ConnectError, Connection, Outbox, Timing};
pub use pending::{Pending, Unanswered};
pub use reconnect::{reconnect, Session};
#[cfg(feature = "tls")]
pub use tls::Trust;
