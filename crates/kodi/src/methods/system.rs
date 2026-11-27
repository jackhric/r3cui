//! Who Kodi is and its settings: for the connection test, the status row and list sorting.

use crate::methods::simple;
use crate::rpc::Method;
use serde::{Deserialize, Serialize};

simple!(/// Liveness on an idle connection.
    Ping = "JSONRPC.Ping");

#[derive(Debug, Clone, Default, Serialize)]
pub struct Version {}
impl Method for Version {
    const NAME: &'static str = "JSONRPC.Version";
    type Output = VersionResult;
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct VersionResult {
    pub version: ApiVersion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct ApiVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct GetApplicationProperties {
    pub properties: &'static [AppProperty],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AppProperty {
    Name,
    Version,
}

impl Method for GetApplicationProperties {
    const NAME: &'static str = "Application.GetProperties";
    type Output = AppProps;
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AppProps {
    pub name: String,
    pub version: AppVersion,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AppVersion {
    pub major: u32,
    pub minor: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct GetSettingValue {
    pub setting: &'static str,
}
impl Method for GetSettingValue {
    const NAME: &'static str = "Settings.GetSettingValue";
    type Output = SettingValue;
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SettingValue {
    pub value: Setting,
}

/// A setting's value: the ones read are booleans; numbers and strings are kept, anything else is `Other`.
#[derive(Debug, Clone, PartialEq)]
pub enum Setting {
    Bool(bool),
    Number(i64),
    Text(String),
    Other,
}

impl<'de> Deserialize<'de> for Setting {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Bool(bool),
            Number(i64),
            Text(String),
            Other(serde::de::IgnoredAny),
        }
        Ok(match Raw::deserialize(d)? {
            Raw::Bool(b) => Self::Bool(b),
            Raw::Number(n) => Self::Number(n),
            Raw::Text(t) => Self::Text(t),
            Raw::Other(_) => Self::Other,
        })
    }
}
