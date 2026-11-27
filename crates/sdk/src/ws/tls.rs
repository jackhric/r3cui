//! TLS for `wss://`: the usual web roots, or, for a device on the local network with a self-signed certificate,
//! whatever certificate it shows.

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{ring, verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Trust {
    #[default]
    Verified,
    /// Encrypted, but anyone on the path could stand in for the server.
    AnyCertificate,
}

pub fn provider() -> Arc<CryptoProvider> {
    Arc::new(ring::default_provider())
}

pub(crate) fn client_config(trust: Trust) -> Result<Arc<ClientConfig>, rustls::Error> {
    let provider = provider();
    let builder = ClientConfig::builder_with_provider(provider.clone()).with_safe_default_protocol_versions()?;
    let config = match trust {
        Trust::Verified => builder.with_root_certificates(RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() }),
        Trust::AnyCertificate => builder.dangerous().with_custom_certificate_verifier(Arc::new(AcceptAnyServer(provider))),
    };
    Ok(Arc::new(config.with_no_client_auth()))
}

/// Checks only that the server holds the key of the certificate it shows.
#[derive(Debug)]
pub struct AcceptAnyServer(pub Arc<CryptoProvider>);

impl ServerCertVerifier for AcceptAnyServer {
    fn verify_server_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ws::connect_trusting;
    use tokio::net::TcpListener;
    use tokio_rustls::TlsAcceptor;

    /// A `wss://` server on a certificate it made itself, as a Home Assistant with a self-signed one has.
    async fn self_signed_server() -> String {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let key = rustls::pki_types::PrivateKeyDer::Pkcs8(cert.signing_key.serialize_der().into());
        let config = rustls::ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert.cert.der().clone()], key)
            .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let Ok(tls) = acceptor.accept(tcp).await else { return };
                    if let Ok(mut ws) = tokio_tungstenite::accept_async(tls).await {
                        use futures_util::StreamExt;
                        while ws.next().await.is_some() {}
                    }
                });
            }
        });
        format!("wss://localhost:{port}")
    }

    #[tokio::test]
    async fn a_self_signed_certificate_is_refused_unless_any_is_trusted() {
        let url = self_signed_server().await;
        let refused = connect_trusting(url.as_str(), Trust::Verified).await.err().expect("refused");
        assert!(refused.reason.to_lowercase().contains("certificate"), "{refused}");
        assert!(connect_trusting(url.as_str(), Trust::AnyCertificate).await.is_ok());
    }
}
