//! TLS as the TV wants it: our self-signed client certificate, and no verification of the TV's own, which is
//! self-signed too. Pairing is what ties the two certificates together, through the code's hash.

use crate::credentials::Credentials;
use crate::endpoint::Endpoint;
use crate::error::{Error, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerifier};
use rustls::crypto::{ring, verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

pub fn provider() -> Arc<CryptoProvider> {
    Arc::new(ring::default_provider())
}

#[derive(Debug)]
struct AcceptAnyServer(Arc<CryptoProvider>);

impl ServerCertVerifier for AcceptAnyServer {
    fn verify_server_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

pub fn client_config(credentials: &Credentials) -> Result<Arc<ClientConfig>> {
    let provider = provider();
    let mut config = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyServer(provider)))
        .with_client_auth_cert(vec![credentials.cert_der()?], credentials.key_der()?)?;
    // A resumed session skips the TV's check of our certificate, which would hide that it has forgotten us.
    config.resumption = rustls::client::Resumption::disabled();
    Ok(Arc::new(config))
}

pub struct Tv {
    pub stream: TlsStream<TcpStream>,
    pub cert: Vec<u8>,
}

pub async fn dial(endpoint: &Endpoint, port: u16, config: Arc<ClientConfig>) -> Result<Tv> {
    let name = ServerName::try_from(endpoint.host.clone()).map_err(|e| Error::Certificate(e.to_string()))?;
    let connecting = async {
        let tcp = TcpStream::connect((endpoint.host.as_str(), port)).await?;
        tcp.set_nodelay(true)?;
        TlsConnector::from(config).connect(name, tcp).await.map_err(Error::from_read)
    };
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, connecting).await.map_err(|_| Error::Timeout)??;
    let cert = stream.get_ref().1.peer_certificates().and_then(|c| c.first()).ok_or(Error::Certificate("the TV sent no certificate".into()))?.to_vec();
    Ok(Tv { stream, cert })
}
