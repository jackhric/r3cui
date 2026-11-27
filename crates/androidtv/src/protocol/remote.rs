//! Remote-channel messages (`remotemessage.proto`, proto3), only the ones this client reads or writes. Fields it
//! never looks at are left out; prost skips them on decode.

use super::keys::{Direction, KeyCode};
use prost::Message;

/// What `Configure.code1` and `SetActive.active` carry: the features both sides speak.
pub mod feature {
    pub const PING: i32 = 1 << 0;
    pub const KEY: i32 = 1 << 1;
    pub const IME: i32 = 1 << 2;
    pub const POWER: i32 = 1 << 5;
    pub const VOLUME: i32 = 1 << 6;
    pub const APP_LINK: i32 = 1 << 9;
    /// IME is what makes the box report the foreground app.
    pub const WANTED: i32 = PING | KEY | IME | POWER | VOLUME | APP_LINK;
}

pub const CLIENT_PACKAGE: &str = "atvremote";
pub const CLIENT_VERSION: &str = "1.0.0";

#[derive(Clone, PartialEq, Message)]
pub struct DeviceInfo {
    #[prost(string, tag = "1")]
    pub model: String,
    #[prost(string, tag = "2")]
    pub vendor: String,
    #[prost(int32, tag = "3")]
    pub unknown1: i32,
    #[prost(string, tag = "4")]
    pub unknown2: String,
    #[prost(string, tag = "5")]
    pub package_name: String,
    #[prost(string, tag = "6")]
    pub app_version: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct Configure {
    #[prost(int32, tag = "1")]
    pub code1: i32,
    #[prost(message, optional, tag = "2")]
    pub device_info: Option<DeviceInfo>,
}

#[derive(Clone, PartialEq, Message)]
pub struct SetActive {
    #[prost(int32, tag = "1")]
    pub active: i32,
}

#[derive(Clone, PartialEq, Message)]
pub struct RemoteError {
    #[prost(bool, tag = "1")]
    pub value: bool,
    #[prost(message, optional, boxed, tag = "2")]
    pub message: Option<Box<RemoteMessage>>,
}

#[derive(Clone, PartialEq, Message)]
pub struct PingRequest {
    #[prost(int32, tag = "1")]
    pub val1: i32,
    #[prost(int32, tag = "2")]
    pub val2: i32,
}

#[derive(Clone, PartialEq, Message)]
pub struct PingResponse {
    #[prost(int32, tag = "1")]
    pub val1: i32,
}

#[derive(Clone, PartialEq, Message)]
pub struct KeyInject {
    #[prost(enumeration = "KeyCode", tag = "1")]
    pub key_code: i32,
    #[prost(enumeration = "Direction", tag = "2")]
    pub direction: i32,
}

#[derive(Clone, PartialEq, Message)]
pub struct AppInfo {
    #[prost(int32, tag = "1")]
    pub counter: i32,
    #[prost(string, tag = "12")]
    pub app_package: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct ImeKeyInject {
    #[prost(message, optional, tag = "1")]
    pub app_info: Option<AppInfo>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Start {
    #[prost(bool, tag = "1")]
    pub started: bool,
}

#[derive(Clone, PartialEq, Message)]
pub struct SetVolumeLevel {
    #[prost(string, tag = "3")]
    pub player_model: String,
    #[prost(uint32, tag = "6")]
    pub volume_max: u32,
    #[prost(uint32, tag = "7")]
    pub volume_level: u32,
    #[prost(bool, tag = "8")]
    pub volume_muted: bool,
}

#[derive(Clone, PartialEq, Message)]
pub struct AppLinkLaunchRequest {
    #[prost(string, tag = "1")]
    pub app_link: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct RemoteMessage {
    #[prost(message, optional, tag = "1")]
    pub configure: Option<Configure>,
    #[prost(message, optional, tag = "2")]
    pub set_active: Option<SetActive>,
    #[prost(message, optional, tag = "3")]
    pub error: Option<RemoteError>,
    #[prost(message, optional, tag = "8")]
    pub ping_request: Option<PingRequest>,
    #[prost(message, optional, tag = "9")]
    pub ping_response: Option<PingResponse>,
    #[prost(message, optional, tag = "10")]
    pub key_inject: Option<KeyInject>,
    #[prost(message, optional, tag = "20")]
    pub ime_key_inject: Option<ImeKeyInject>,
    #[prost(message, optional, tag = "40")]
    pub start: Option<Start>,
    #[prost(message, optional, tag = "50")]
    pub set_volume_level: Option<SetVolumeLevel>,
    #[prost(message, optional, tag = "90")]
    pub app_link_launch_request: Option<AppLinkLaunchRequest>,
}

impl RemoteMessage {
    pub fn key(key: KeyCode, direction: Direction) -> Self {
        Self { key_inject: Some(KeyInject { key_code: key as i32, direction: direction as i32 }), ..Self::default() }
    }

    pub fn launch(app_link: &str) -> Self {
        Self { app_link_launch_request: Some(AppLinkLaunchRequest { app_link: app_link.into() }), ..Self::default() }
    }

    pub fn pong(request: &PingRequest) -> Self {
        Self { ping_response: Some(PingResponse { val1: request.val1 }), ..Self::default() }
    }

    pub fn configure(features: i32, device: DeviceInfo) -> Self {
        Self { configure: Some(Configure { code1: features, device_info: Some(device) }), ..Self::default() }
    }

    pub fn set_active(features: i32) -> Self {
        Self { set_active: Some(SetActive { active: features }), ..Self::default() }
    }
}

impl DeviceInfo {
    /// What this client says about itself in its `Configure` reply.
    pub fn this_client() -> Self {
        Self { unknown1: 1, unknown2: "1".into(), package_name: CLIENT_PACKAGE.into(), app_version: CLIENT_VERSION.into(), ..Self::default() }
    }
}
