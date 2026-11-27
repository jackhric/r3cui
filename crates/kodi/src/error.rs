//! The crate's one error type. `Clone`, so it can sit in `KodiState` for the UI to show.

use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("Kodi is not set up")]
    NotConfigured,
    #[error("cannot reach {endpoint}: {reason}")]
    Unreachable { endpoint: Arc<str>, reason: Arc<str> },
    #[error("{method} timed out after {after:?}")]
    Timeout { method: &'static str, after: Duration },
    /// Kodi's own JSON-RPC error: -32602 invalid params, -32100 failed to execute, -32601 no such method.
    #[error("{method}: Kodi error {code}: {message}")]
    Rpc { method: &'static str, code: i32, message: Arc<str> },
    #[error("{method}: unexpected reply: {detail}")]
    Decode { method: &'static str, detail: Arc<str> },
    #[error("nothing is playing")]
    NothingPlaying,
    #[error("connection closed")]
    Closed,
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<r3cui_sdk::ws::ConnectError> for Error {
    fn from(e: r3cui_sdk::ws::ConnectError) -> Self {
        Self::Unreachable { endpoint: e.url.into(), reason: e.reason.into() }
    }
}
