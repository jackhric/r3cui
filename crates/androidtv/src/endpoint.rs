pub const REMOTE_PORT: u16 = 6466;
pub const PAIRING_PORT: u16 = 6467;

/// Where a TV is: an IP address or a name, and the two ports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub remote_port: u16,
    pub pairing_port: u16,
}

impl Endpoint {
    pub fn new(host: impl Into<String>) -> Self {
        Self { host: host.into(), remote_port: REMOTE_PORT, pairing_port: PAIRING_PORT }
    }

    pub fn with_ports(mut self, remote_port: u16, pairing_port: u16) -> Self {
        self.remote_port = remote_port;
        self.pairing_port = pairing_port;
        self
    }
}

impl From<&str> for Endpoint {
    fn from(host: &str) -> Self {
        Self::new(host)
    }
}

impl From<String> for Endpoint {
    fn from(host: String) -> Self {
        Self::new(host)
    }
}
