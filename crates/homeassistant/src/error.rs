//! The crate's one error type. `Clone`, so it can sit in `HaState` for the UI to show.

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("Home Assistant is not set up")]
    NotConfigured,
    #[error("cannot reach Home Assistant: {0}")]
    Unreachable(String),
    #[error("Home Assistant refused the access token: {0}")]
    Rejected(String),
    #[error("Home Assistant did not answer in time")]
    Timeout,
    #[error("Home Assistant error {code}: {message}")]
    Failed { code: String, message: String },
    #[error("unexpected reply: {0}")]
    Decode(String),
    #[error("connection closed")]
    Closed,
    #[error("{0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<wsclient::ConnectError> for Error {
    fn from(e: wsclient::ConnectError) -> Self {
        Self::Unreachable(e.to_string())
    }
}
