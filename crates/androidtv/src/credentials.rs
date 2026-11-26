use crate::error::{Error, Result};
use rand_core::OsRng;
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, PKCS_RSA_SHA256};
use rsa::pkcs8::EncodePrivateKey;
use rsa::RsaPrivateKey;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde::{Deserialize, Serialize};

const KEY_BITS: usize = 2048;
const COMMON_NAME: &str = "r3cui";

/// The client certificate the TV pairs with and recognises afterwards. Keep it: a new one means pairing again.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credentials {
    pub cert_pem: String,
    pub key_pem: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials").finish_non_exhaustive()
    }
}

impl Credentials {
    /// A fresh RSA key and self-signed certificate. The key generation is CPU-bound (seconds on the device), so
    /// call it from `spawn_blocking`.
    pub fn generate() -> Result<Self> {
        let rsa = RsaPrivateKey::new(&mut OsRng, KEY_BITS).map_err(|e| Error::Certificate(e.to_string()))?;
        let pkcs8 = rsa.to_pkcs8_der().map_err(|e| Error::Certificate(e.to_string()))?;
        let key = KeyPair::from_pkcs8_der_and_sign_algo(&PrivatePkcs8KeyDer::from(pkcs8.as_bytes()), &PKCS_RSA_SHA256).map_err(cert_error)?;
        let mut params = CertificateParams::new(vec![COMMON_NAME.to_string()]).map_err(cert_error)?;
        params.distinguished_name.push(DnType::CommonName, COMMON_NAME);
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        let cert = params.self_signed(&key).map_err(cert_error)?;
        Ok(Self { cert_pem: cert.pem(), key_pem: key.serialize_pem() })
    }

    pub fn cert_der(&self) -> Result<CertificateDer<'static>> {
        CertificateDer::from_pem_slice(self.cert_pem.as_bytes()).map_err(|e| Error::Certificate(e.to_string()))
    }

    pub(crate) fn key_der(&self) -> Result<PrivateKeyDer<'static>> {
        PrivateKeyDer::from_pem_slice(self.key_pem.as_bytes()).map_err(|e| Error::Certificate(e.to_string()))
    }
}

fn cert_error(e: rcgen::Error) -> Error {
    Error::Certificate(e.to_string())
}
