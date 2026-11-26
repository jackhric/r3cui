//! The pairing secret: SHA-256 over both certificates' RSA public numbers and the code's last two bytes. The
//! first byte of the hash doubles as the code's first byte, so a mistyped code is caught before it is sent.

use crate::error::{Error, Result};
use sha2::{Digest, Sha256};
use x509_parser::prelude::{FromDer, X509Certificate};
use x509_parser::public_key::PublicKey;

const CODE_LEN: usize = 6;

/// The six hexadecimal symbols the TV shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Code {
    pub check: u8,
    pub tail: [u8; 2],
}

impl Code {
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        if text.len() != CODE_LEN || !text.is_ascii() {
            return Err(Error::InvalidCode);
        }
        let byte = |i: usize| u8::from_str_radix(&text[i..i + 2], 16).map_err(|_| Error::InvalidCode);
        Ok(Self { check: byte(0)?, tail: [byte(2)?, byte(4)?] })
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn display(&self) -> String {
        format!("{:02X}{:02X}{:02X}", self.check, self.tail[0], self.tail[1])
    }
}

/// An RSA public key as big-endian bytes without leading zeros, which is how the hash wants them.
struct RsaNumbers {
    modulus: Vec<u8>,
    exponent: Vec<u8>,
}

fn rsa_numbers(cert_der: &[u8]) -> Result<RsaNumbers> {
    let bad = |what: &str| Error::Certificate(what.to_string());
    let (_, cert) = X509Certificate::from_der(cert_der).map_err(|e| bad(&e.to_string()))?;
    let PublicKey::RSA(key) = cert.public_key().parsed().map_err(|e| bad(&e.to_string()))? else {
        return Err(bad("not an RSA key"));
    };
    let trim = |n: &[u8]| n.iter().skip_while(|b| **b == 0).copied().collect();
    Ok(RsaNumbers { modulus: trim(key.modulus), exponent: trim(key.exponent) })
}

/// The hash both sides compute: it is the secret, and its first byte is the code's check byte.
pub fn digest(client_cert: &[u8], server_cert: &[u8], tail: [u8; 2]) -> Result<[u8; 32]> {
    let (client, server) = (rsa_numbers(client_cert)?, rsa_numbers(server_cert)?);
    let mut hash = Sha256::new();
    for part in [&client.modulus, &client.exponent, &server.modulus, &server.exponent] {
        hash.update(part);
    }
    hash.update(tail);
    Ok(hash.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_parse_and_print() {
        let code = Code::parse(" 1a2B3c ").unwrap();
        assert_eq!(code, Code { check: 0x1a, tail: [0x2b, 0x3c] });
        assert_eq!(code.display(), "1A2B3C");
    }

    #[test]
    fn bad_codes_are_refused() {
        for text in ["", "12345", "1234567", "12345g", "é2345"] {
            assert!(matches!(Code::parse(text), Err(Error::InvalidCode)), "{text:?}");
        }
    }
}
