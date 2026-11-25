//! Everything r3cui talks to, one module each: Kodi (and its poster wall), the receiver and the key-backlight
//! helpers in Home Assistant, the remote's own system (power, battery, settings), its Wi-Fi, and the links' health.

pub(crate) mod ha_keys;
pub(crate) mod health;
pub(crate) mod kodi_direct;
pub(crate) mod library;
pub(crate) mod receiver;
pub(crate) mod system;
pub(crate) mod wifi;
