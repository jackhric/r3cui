//! Pairing messages (`polo.proto`, proto2: its `required` fields are always written).

use prost::{Enumeration, Message};

pub const SERVICE_NAME: &str = "atvremote";
pub const PROTOCOL_VERSION: u32 = 2;
/// The pairing code is this many hexadecimal symbols; the first two are the check byte.
pub const CODE_SYMBOLS: u32 = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Enumeration)]
#[repr(i32)]
pub enum Status {
    Ok = 200,
    Error = 400,
    BadConfiguration = 401,
    BadSecret = 402,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Enumeration)]
#[repr(i32)]
pub enum EncodingType {
    Unknown = 0,
    Alphanumeric = 1,
    Numeric = 2,
    Hexadecimal = 3,
    Qrcode = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Enumeration)]
#[repr(i32)]
pub enum RoleType {
    Unknown = 0,
    Input = 1,
    Output = 2,
}

#[derive(Clone, PartialEq, Message)]
pub struct OuterMessage {
    #[prost(uint32, required, tag = "1")]
    pub protocol_version: u32,
    #[prost(int32, required, tag = "2")]
    pub status: i32,
    #[prost(message, optional, tag = "10")]
    pub pairing_request: Option<PairingRequest>,
    #[prost(message, optional, tag = "11")]
    pub pairing_request_ack: Option<PairingRequestAck>,
    #[prost(message, optional, tag = "20")]
    pub options: Option<Options>,
    #[prost(message, optional, tag = "30")]
    pub configuration: Option<Configuration>,
    #[prost(message, optional, tag = "31")]
    pub configuration_ack: Option<ConfigurationAck>,
    #[prost(message, optional, tag = "40")]
    pub secret: Option<Secret>,
    #[prost(message, optional, tag = "41")]
    pub secret_ack: Option<SecretAck>,
}

#[derive(Clone, PartialEq, Message)]
pub struct PairingRequest {
    #[prost(string, required, tag = "1")]
    pub service_name: String,
    #[prost(string, optional, tag = "2")]
    pub client_name: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
pub struct PairingRequestAck {
    #[prost(string, optional, tag = "1")]
    pub server_name: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Encoding {
    #[prost(enumeration = "EncodingType", required, tag = "1")]
    pub r#type: i32,
    #[prost(uint32, required, tag = "2")]
    pub symbol_length: u32,
}

#[derive(Clone, PartialEq, Message)]
pub struct Options {
    #[prost(message, repeated, tag = "1")]
    pub input_encodings: Vec<Encoding>,
    #[prost(message, repeated, tag = "2")]
    pub output_encodings: Vec<Encoding>,
    #[prost(enumeration = "RoleType", optional, tag = "3")]
    pub preferred_role: Option<i32>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Configuration {
    #[prost(message, required, tag = "1")]
    pub encoding: Encoding,
    #[prost(enumeration = "RoleType", required, tag = "2")]
    pub client_role: i32,
}

#[derive(Clone, PartialEq, Message)]
pub struct ConfigurationAck {}

#[derive(Clone, PartialEq, Message)]
pub struct Secret {
    #[prost(bytes = "vec", required, tag = "1")]
    pub secret: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct SecretAck {
    #[prost(bytes = "vec", required, tag = "1")]
    pub secret: Vec<u8>,
}

impl OuterMessage {
    pub fn ok() -> Self {
        Self::with_status(Status::Ok)
    }

    pub fn with_status(status: Status) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            status: status as i32,
            pairing_request: None,
            pairing_request_ack: None,
            options: None,
            configuration: None,
            configuration_ack: None,
            secret: None,
            secret_ack: None,
        }
    }

    pub fn status(&self) -> Option<Status> {
        Status::try_from(self.status).ok()
    }
}

impl Encoding {
    pub fn hex_code() -> Self {
        Self { r#type: EncodingType::Hexadecimal as i32, symbol_length: CODE_SYMBOLS }
    }
}
