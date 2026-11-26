//! What answers for the remote itself: the core, or in the demo a made-up remote with a Wi-Fi to join.

use crate::app::Later;
use crate::hardware::backlight::Backlight;
use crate::remote::system;
use crate::remote::wifi::{self, Network, Wifi};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone)]
pub(crate) enum Link {
    Core(coreapi::Core),
    #[cfg(feature = "demo")]
    Demo(crate::app::demo::Net),
}

impl Link {
    pub(crate) fn backlight(&self) -> Option<Backlight> {
        match self {
            Self::Core(core) => Some(Backlight::start(core.clone())),
            #[cfg(feature = "demo")]
            Self::Demo(_) => None,
        }
    }

    /// How long to wait before looking again at the Wi-Fi: the core's `wait`, or less in the demo.
    pub(crate) fn settle(&self, wait: Duration) -> Duration {
        match self {
            Self::Core(_) => wait,
            #[cfg(feature = "demo")]
            Self::Demo(_) => wait.min(crate::app::demo::WIFI_SETTLE),
        }
    }

    /// The network the remote is on, and what it can see.
    pub(crate) async fn scan(&self) -> (String, Vec<Network>) {
        match self {
            Self::Core(core) => wifi::scan(core).await,
            #[cfg(feature = "demo")]
            Self::Demo(net) => net.scan(),
        }
    }

    pub(crate) fn start_scan(&self, wifi: &mut Wifi) {
        match self {
            Self::Core(core) => wifi.start_scan(core),
            #[cfg(feature = "demo")]
            Self::Demo(_) => wifi.scanning = true,
        }
    }

    pub(crate) async fn toggle_band(&self, wifi: &mut Wifi) {
        match self {
            Self::Core(core) => wifi.toggle_band(core).await,
            #[cfg(feature = "demo")]
            Self::Demo(net) => wifi.band = net.toggle_band(),
        }
    }

    /// Whether the join started; if not, the caller reports the failure itself. The demo answers through `later`.
    #[cfg_attr(not(feature = "demo"), allow(unused_variables))]
    pub(crate) async fn join(&self, wifi: &mut Wifi, ssid: &str, hex: &str, password: &str, later: &UnboundedSender<Later>) -> bool {
        match self {
            Self::Core(core) => wifi.join(core, ssid, hex, password).await,
            #[cfg(feature = "demo")]
            Self::Demo(net) => {
                wifi.begin_join();
                net.join(ssid, later.clone());
                true
            }
        }
    }

    pub(crate) fn set_display(&self, brightness: i32, auto: bool) {
        match self {
            Self::Core(core) => system::set_display(core, brightness, auto),
            #[cfg(feature = "demo")]
            Self::Demo(_) => {}
        }
    }
}
