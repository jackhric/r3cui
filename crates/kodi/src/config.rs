//! Where Kodi is: host, ports and the web server's credentials, and the URLs built from them.

use crate::art::encode;
use crate::wire::ArtRef;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KodiConfig {
    /// A name or an IP address.
    pub host: String,
    /// The web server: JSON-RPC over HTTP, and artwork.
    #[serde(default = "http_port")]
    pub http_port: u16,
    /// JSON-RPC over WebSocket, which carries Kodi's notifications. Only `advancedsettings.xml` can change it.
    #[serde(default = "ws_port")]
    pub ws_port: u16,
    /// The web server's login. The WebSocket has none: Kodi doesn't authenticate it at all.
    #[serde(default)]
    pub credentials: Option<Credentials>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

fn http_port() -> u16 {
    8080
}

fn ws_port() -> u16 {
    9090
}

impl KodiConfig {
    pub fn new(host: impl Into<String>) -> Self {
        Self { host: host.into(), http_port: http_port(), ws_port: ws_port(), credentials: None }
    }

    pub fn ws_url(&self) -> String {
        format!("ws://{}:{}/jsonrpc", self.host, self.ws_port)
    }

    pub fn http_base(&self) -> String {
        format!("http://{}:{}", self.host, self.http_port)
    }

    /// Where Kodi serves an artwork reference (from its cache, fetching it first if need be).
    pub fn image_url(&self, art: &ArtRef) -> String {
        format!("{}/image/{}", self.http_base(), encode(&art.0))
    }
}
