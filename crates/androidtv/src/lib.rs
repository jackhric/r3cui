//! r3cui's own client for the Android TV Remote protocol v2, the one Google's phone remote app speaks: no developer
//! options and no ADB, just pairing once with a code shown on the TV. The design is in docs/androidtv-client.md.
//!
//! `Credentials::generate` makes the client certificate, `pair` and `Pairing::finish` introduce it to the TV, and
//! `connect` follows the TV on the remote channel and returns an `AndroidTvHandle`.
//!
//! The protobuf field numbers in `protocol` come from `polo.proto` and `remotemessage.proto` of androidtvremote2
//! (github.com/tronikos/androidtvremote2, Apache-2.0), which in turn derive from Google's pairing protocol and the
//! louis49/androidtv-remote project. The flow follows that library's `pairing.py` and `remote.py`.

mod channel;
mod credentials;
mod endpoint;
mod error;
mod pairing;
pub mod protocol;
mod secret;
mod session;
mod state;
#[cfg(feature = "testing")]
pub mod testing;
mod tls;

pub use credentials::Credentials;
pub use endpoint::{Endpoint, PAIRING_PORT, REMOTE_PORT};
pub use error::{Error, Result};
pub use pairing::{pair, Pairing};
pub use protocol::{Direction, KeyCode};
pub use session::{connect, AndroidTvHandle};
pub use state::{Link, TvState, Volume};
