//! Pairing: introduce this client's certificate to the TV, which then shows a code to type back in.

use crate::channel::Channel;
use crate::credentials::Credentials;
use crate::endpoint::Endpoint;
use crate::error::{Error, Result};
use crate::protocol::polo::{Configuration, Encoding, Options, OuterMessage, PairingRequest, RoleType, Secret, Status, SERVICE_NAME};
use crate::secret::{digest, Code};
use crate::tls::{client_config, dial};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

/// Every step before the code is answered by the TV itself, not a person.
const STEP_TIMEOUT: Duration = Duration::from_secs(10);

/// A pairing in progress: the TV is showing a code. Hand it to `finish`.
pub struct Pairing {
    channel: Channel<TlsStream<TcpStream>>,
    client_cert: Vec<u8>,
    server_cert: Vec<u8>,
}

/// Connects to the TV's pairing port and runs the handshake up to the TV displaying its code.
pub async fn pair(endpoint: impl Into<Endpoint>, credentials: &Credentials, client_name: &str) -> Result<Pairing> {
    let endpoint = endpoint.into();
    let tv = dial(&endpoint, endpoint.pairing_port, client_config(credentials)?).await?;
    let mut channel = Channel::new(tv.stream);

    let mut request = OuterMessage::ok();
    request.pairing_request = Some(PairingRequest { service_name: SERVICE_NAME.into(), client_name: Some(client_name.into()) });
    channel.send(&request).await?;
    loop {
        let message = step(&mut channel).await?;
        let mut reply = OuterMessage::ok();
        if message.pairing_request_ack.is_some() {
            reply.options = Some(Options { input_encodings: vec![Encoding::hex_code()], output_encodings: vec![], preferred_role: Some(RoleType::Input as i32) });
        } else if message.options.is_some() {
            reply.configuration = Some(Configuration { encoding: Encoding::hex_code(), client_role: RoleType::Input as i32 });
        } else if message.configuration_ack.is_some() {
            return Ok(Pairing { channel, client_cert: credentials.cert_der()?.to_vec(), server_cert: tv.cert });
        } else {
            return Err(Error::Unexpected("during pairing setup"));
        }
        channel.send(&reply).await?;
    }
}

async fn step(channel: &mut Channel<TlsStream<TcpStream>>) -> Result<OuterMessage> {
    let message: OuterMessage = tokio::time::timeout(STEP_TIMEOUT, channel.recv()).await.map_err(|_| Error::Timeout)??;
    match message.status() {
        Some(Status::Ok) => Ok(message),
        Some(Status::BadSecret) => Err(Error::WrongCode),
        Some(other) => Err(Error::Refused(other)),
        None => Err(Error::Unexpected("status")),
    }
}

impl Pairing {
    /// Sends the secret for the code the TV shows. A code that cannot be right is refused here without bothering
    /// the TV, and can be retyped; once the TV itself says no, pair again.
    pub async fn finish(&mut self, code: &str) -> Result<()> {
        let code = Code::parse(code)?;
        let secret = digest(&self.client_cert, &self.server_cert, code.tail)?;
        if secret[0] != code.check {
            return Err(Error::WrongCode);
        }
        let mut message = OuterMessage::ok();
        message.secret = Some(Secret { secret: secret.to_vec() });
        self.channel.send(&message).await?;
        let ack = step(&mut self.channel).await?;
        ack.secret_ack.map(|_| ()).ok_or(Error::Unexpected("instead of the secret acknowledgement"))
    }
}

