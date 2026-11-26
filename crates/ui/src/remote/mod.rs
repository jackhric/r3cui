//! The remote itself, through the core: its system (power, battery, settings), its Wi-Fi, and how every link the
//! screen depends on is doing. Other programs are drivers (`crates/drivers`), not here.

pub(crate) mod health;
pub(crate) mod link;
pub(crate) mod selfupdate;
pub(crate) mod software;
pub(crate) mod system;
pub(crate) mod wifi;
