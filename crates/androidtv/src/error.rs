use crate::protocol::polo::Status;
use std::io;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O: {0}")]
    Io(#[from] io::Error),
    #[error("TLS: {0}")]
    Tls(#[from] rustls::Error),
    #[error("certificate: {0}")]
    Certificate(String),
    #[error("not connected")]
    NotConnected,
    #[error("the TV closed the connection")]
    Closed,
    #[error("no answer from the TV in time")]
    Timeout,
    #[error("the TV refused our certificate: this client has to be paired first")]
    NotPaired,
    #[error("the pairing code must be six hexadecimal characters")]
    InvalidCode,
    #[error("wrong pairing code")]
    WrongCode,
    #[error("pairing refused by the TV: status {0:?}")]
    Refused(Status),
    #[error("unexpected message from the TV: {0}")]
    Unexpected(&'static str),
    #[error("malformed message: {0}")]
    Decode(#[from] prost::DecodeError),
    #[error("message of {0} bytes is over the limit")]
    TooLarge(usize),
}

impl Error {
    /// A TLS alert from the TV during or right after the handshake is how an unpaired client finds out.
    pub(crate) fn from_read(e: io::Error) -> Self {
        let alert = e.get_ref().and_then(|inner| inner.downcast_ref::<rustls::Error>()).is_some_and(|t| matches!(t, rustls::Error::AlertReceived(_)));
        if alert {
            Error::NotPaired
        } else {
            Error::Io(e)
        }
    }
}
