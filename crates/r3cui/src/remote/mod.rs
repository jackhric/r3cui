//! The remote itself, through the core: its system (power, battery, settings), its Wi-Fi, and how every link the
//! screen depends on is doing. Other programs are integrations (`docs/drivers.md`), not here.

pub(crate) mod health;
pub(crate) mod link;
pub(crate) mod rest;
pub(crate) mod selfupdate;
pub(crate) mod software;
pub(crate) mod system;
pub(crate) mod voice;
pub(crate) mod voicesetup;
pub(crate) mod wifi;
