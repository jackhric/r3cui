//! What answers for the remote itself: the core, or in the demo a made-up remote with a Wi-Fi to join.

use crate::app::software::Access;
use crate::app::Later;
use crate::hardware::backlight::Backlight;
use crate::remote::selfupdate::Release;
use crate::remote::software::{self, Progress};
use crate::remote::system;
use crate::remote::voice;
use crate::remote::voicesetup;
use crate::remote::wifi::{self, Network, Wifi};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone)]
pub(crate) enum Link {
    Core(coreapi::Core),
    #[cfg(feature = "demo")]
    Demo(crate::app::demo::Remote),
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

    pub(crate) async fn firmware_info(&self, force: bool) -> Result<coreapi::SystemUpdateInfo, String> {
        match self {
            Self::Core(core) => software::firmware_info(core, force).await,
            #[cfg(feature = "demo")]
            Self::Demo(remote) => Ok(remote.updates().firmware_info()),
        }
    }

    #[cfg_attr(not(feature = "demo"), allow(unused_variables))]
    pub(crate) async fn start_firmware(&self, id: &str, later: &UnboundedSender<Later>) -> Result<coreapi::UpdateStart, coreapi::UpdateRefusal> {
        match self {
            Self::Core(core) => software::start_firmware(core, id).await,
            #[cfg(feature = "demo")]
            Self::Demo(remote) => remote.updates().start_firmware(later),
        }
    }

    pub(crate) async fn web_enabled(&self) -> Result<bool, String> {
        match self {
            Self::Core(core) => software::web_enabled(core).await,
            #[cfg(feature = "demo")]
            Self::Demo(remote) => Ok(remote.updates().web_enabled()),
        }
    }

    /// Turns the web configurator on with this PIN, or off.
    pub(crate) async fn set_web(&self, pin: Option<&str>) -> Result<(), String> {
        match self {
            Self::Core(core) => software::set_web(core, pin).await,
            #[cfg(feature = "demo")]
            Self::Demo(remote) => {
                remote.updates().set_web(pin.is_some());
                Ok(())
            }
        }
    }

    pub(crate) async fn address(&self) -> String {
        match self {
            Self::Core(core) => software::address(core).await,
            #[cfg(feature = "demo")]
            Self::Demo(_) => demo::data::REMOTE_ADDRESS.into(),
        }
    }

    /// The newest r3cui release, if it is newer than the one running.
    pub(crate) async fn latest_release(&self) -> Result<Option<Release>, String> {
        match self {
            Self::Core(_) => software::latest_release().await,
            #[cfg(feature = "demo")]
            Self::Demo(_) => Ok(Some(crate::app::demo::software::release())),
        }
    }

    pub(crate) async fn install_r3(&self, release: Release, access: Access, progress: Progress) -> Result<(), String> {
        match self {
            Self::Core(core) => software::install_r3(core, release, access, progress).await,
            #[cfg(feature = "demo")]
            Self::Demo(_) => crate::app::demo::software::install_r3(progress).await,
        }
    }

    pub(crate) async fn uninstall(&self, access: Access) -> Result<(), String> {
        match self {
            Self::Core(core) => software::uninstall(core, access).await,
            #[cfg(feature = "demo")]
            Self::Demo(_) => crate::app::demo::software::uninstall().await,
        }
    }

    /// Whether r3cui's voice integration is the remote's voice assistant, with the microphone on.
    pub(crate) async fn voice_ready(&self) -> Result<bool, String> {
        match self {
            Self::Core(core) => voicesetup::state(core).await.map(|s| s == voicesetup::State::Ready),
            #[cfg(feature = "demo")]
            Self::Demo(remote) => Ok(remote.updates().voice_ready()),
        }
    }

    pub(crate) async fn install_voice(&self, access: Access) -> Result<(), String> {
        match self {
            Self::Core(core) => voicesetup::install(core, access).await,
            #[cfg(feature = "demo")]
            Self::Demo(remote) => crate::app::demo::software::install_voice(remote.updates()).await,
        }
    }

    /// r3cui's relay, which brings the remote's recorded audio; the demo has no microphone.
    pub(crate) fn relay(&self) -> Option<voicelink::Link> {
        match self {
            Self::Core(_) => Some(voicelink::connect_loop()),
            #[cfg(feature = "demo")]
            Self::Demo(_) => None,
        }
    }

    /// How long the relay gets to end a session's audio after the key is let go: none without one.
    pub(crate) fn voice_grace(&self) -> Duration {
        match self {
            Self::Core(_) => crate::app::voice::END_GRACE,
            #[cfg(feature = "demo")]
            Self::Demo(_) => Duration::ZERO,
        }
    }

    pub(crate) async fn voice_start(&self, session: u32) {
        match self {
            Self::Core(core) => voice::start(core, session).await,
            #[cfg(feature = "demo")]
            Self::Demo(_) => crate::app::demo::voice::talk(session),
        }
    }

    pub(crate) async fn voice_end(&self, session: u32) {
        match self {
            Self::Core(core) => voice::end(core, session).await,
            #[cfg(feature = "demo")]
            Self::Demo(_) => crate::app::demo::voice::stop(session),
        }
    }

    /// Out of standby, as a key press does: the core only hears real keys.
    pub(crate) fn wake(&self) {
        match self {
            Self::Core(core) => {
                let core = core.clone();
                tokio::spawn(async move {
                    if let Err(e) = core.set_power_mode(coreapi::PowerMode::Normal).await {
                        tracing::warn!("waking the remote: {e}");
                    }
                });
            }
            #[cfg(feature = "demo")]
            Self::Demo(_) => {}
        }
    }
}
