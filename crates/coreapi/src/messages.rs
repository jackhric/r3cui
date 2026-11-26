//! The core's message names, once. Requests and events elsewhere in the crate use these, never the strings.

pub const AUTH: &str = "auth";
pub const AUTHENTICATION: &str = "authentication";
pub const AUTH_REQUIRED: &str = "auth_required";
pub const SUBSCRIBE_EVENTS: &str = "subscribe_events";

pub const VERSION: &str = "version";
pub const SET_UI_INFO: &str = "set_ui_info";
pub const CHECK_SYSTEM_UPDATE: &str = "check_system_update";
pub const UPDATE_SYSTEM: &str = "update_system";
pub const GET_API_ACCESS: &str = "get_api_access";
pub const SET_API_ACCESS: &str = "set_api_access";

pub const POWER_MODE_CHANGE: &str = "power_mode_change";
pub const BATTERY_STATUS: &str = "battery_status";
pub const WIFI_CHANGE: &str = "wifi_change";
pub const SOFTWARE_UPDATE: &str = "software_update";
