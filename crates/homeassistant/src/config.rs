//! Where Home Assistant is, and the long-lived access token that lets us in.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HaConfig {
    /// The base address, like `http://homeassistant.local:8123`.
    pub url: String,
    pub token: String,
    /// Take whatever certificate an `https://` Home Assistant shows: for one with a self-signed certificate.
    #[serde(default)]
    pub self_signed: bool,
}

impl fmt::Debug for HaConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HaConfig").field("url", &self.url).field("token", &"..").field("self_signed", &self.self_signed).finish()
    }
}

impl HaConfig {
    pub fn new(url: impl Into<String>, token: impl Into<String>) -> Self {
        Self { url: url.into(), token: token.into(), self_signed: false }
    }

    pub fn self_signed(self, self_signed: bool) -> Self {
        Self { self_signed, ..self }
    }

    pub fn trust(&self) -> wsclient::Trust {
        if self.self_signed { wsclient::Trust::AnyCertificate } else { wsclient::Trust::Verified }
    }

    pub fn is_configured(&self) -> bool {
        !self.url.trim().is_empty() && !self.token.trim().is_empty()
    }

    /// The WebSocket endpoint: `wss://` for an `https://` address, `ws://` otherwise.
    pub fn ws_url(&self) -> Result<String> {
        if !self.is_configured() {
            return Err(Error::NotConfigured);
        }
        let url = self.url.trim().trim_end_matches('/');
        let (scheme, rest) = match url.split_once("://") {
            Some(("http" | "ws", rest)) => ("ws", rest),
            Some(("https" | "wss", rest)) => ("wss", rest),
            Some((scheme, _)) => return Err(Error::Unsupported(format!("unknown scheme {scheme}://"))),
            None => ("ws", url),
        };
        Ok(format!("{scheme}://{rest}/api/websocket"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_url_from_the_base_address() {
        let url = |u: &str| HaConfig::new(u, "t").ws_url();
        assert_eq!(url("http://homeassistant.local:8123").unwrap(), "ws://homeassistant.local:8123/api/websocket");
        assert_eq!(url("http://10.0.0.2:8123/").unwrap(), "ws://10.0.0.2:8123/api/websocket");
        assert_eq!(url("ha.lan:8123").unwrap(), "ws://ha.lan:8123/api/websocket");
        assert_eq!(url("https://homeassistant.internal:8123").unwrap(), "wss://homeassistant.internal:8123/api/websocket");
        assert_eq!(url("wss://ha.example.org").unwrap(), "wss://ha.example.org/api/websocket");
        assert!(matches!(url("ftp://ha.example.org"), Err(Error::Unsupported(_))));
        assert_eq!(HaConfig::new("", "t").ws_url(), Err(Error::NotConfigured));
        assert_eq!(HaConfig::new("http://ha", "").ws_url(), Err(Error::NotConfigured));
    }

    #[test]
    fn debug_hides_the_token() {
        assert!(!format!("{:?}", HaConfig::new("http://ha", "secret")).contains("secret"));
    }
}
