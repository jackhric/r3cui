//! The demo: the real session, with the Demo driver (`integration`) in every slot and a made-up remote in place of
//! the core, so there is nothing to connect to. The made-up data itself is `data`.

pub(crate) mod data;
pub(crate) mod integration;
pub(crate) mod software;
pub(crate) mod voice;

use crate::app::session::{serve, Frontend};
use crate::app::{Later, UiAction};
use crate::art::{backdrop_pixels, poster_pixels, vibrant};
use crate::remote::link::Link;
use crate::remote::system::System;
use crate::remote::wifi::{demo_networks, Network, Wifi};
use crate::view::set_poster_accent;
use crate::{AppWindow, Playback, Prefs};
use crate::app::demo::data::{WIFI_BAND, WIFI_HOME};
use slint::ComponentHandle;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

/// How long the made-up Wi-Fi takes to settle after a scan or a band switch.
pub(crate) const WIFI_SETTLE: Duration = Duration::from_millis(1400);
const JOIN_TIME: Duration = Duration::from_millis(1800);

pub(crate) fn apply_demo_settings(app: &AppWindow) {
    // Artwork for the demo's film, from local files, so previews show the real layout.
    let art = |var: &str| image::open(std::env::var_os(var)?).map_err(|e| tracing::warn!("{var}: {e}")).ok();
    let poster = art("UC_DEMO_POSTER");
    if let Some(p) = &poster {
        app.global::<Playback>().set_poster(slint::Image::from_rgba8(poster_pixels(p)));
        set_poster_accent(app, vibrant::accent(p));
    }
    if let Some(b) = art("UC_DEMO_FANART").or(poster) {
        app.global::<Playback>().set_backdrop(slint::Image::from_rgba8(backdrop_pixels(&b)));
    }
    app.global::<Prefs>().set_haptics("On".into());
    app.global::<Prefs>().set_screen_off_after("60 s".into());
    app.global::<Prefs>().set_sleep_after("3 min".into());
}

/// The made-up remote: its Wi-Fi, and the software it has updates waiting for.
#[derive(Clone)]
pub(crate) struct Remote(Arc<Mutex<NetState>>, software::Updates);

struct NetState {
    ssid: String,
    band: String,
    /// The last network a join was tried on: the first try at a new one fails, so the failure page can be seen, and
    /// the second works.
    tried: String,
}

impl Remote {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(NetState { ssid: WIFI_HOME.into(), band: WIFI_BAND.into(), tried: String::new() })), software::Updates::default())
    }

    pub(crate) fn updates(&self) -> &software::Updates {
        &self.1
    }

    pub(crate) fn scan(&self) -> (String, Vec<Network>) {
        let ssid = self.0.lock().unwrap().ssid.clone();
        let networks = demo_networks(&ssid);
        (ssid, networks)
    }

    pub(crate) fn toggle_band(&self) -> String {
        let mut net = self.0.lock().unwrap();
        net.band = if net.band == "a" { "b".into() } else { "a".into() };
        net.band.clone()
    }

    pub(crate) fn join(&self, ssid: &str, later: mpsc::UnboundedSender<Later>) {
        let mut net = self.0.lock().unwrap();
        let ok = net.tried == ssid;
        net.tried = ssid.to_string();
        if ok {
            net.ssid = ssid.to_string();
        }
        tokio::spawn(async move {
            tokio::time::sleep(JOIN_TIME).await;
            let _ = later.send(Later::JoinDone(ok));
        });
    }
}

/// Keys that act as the remote and its drivers would: `c` on and off the charger, `z` asleep and awake, `x` every
/// driver unreachable and back.
pub(crate) fn toggle(key: char, system: &mut System, links_down: &mut bool) {
    match key {
        'c' => system.charging = !system.charging,
        'z' => system.awake = !system.awake,
        'x' => *links_down = !*links_down,
        _ => {}
    }
}

/// The real session over the demo drivers and a made-up remote.
pub(crate) async fn run(weak: &slint::Weak<AppWindow>, actions: &mut mpsc::UnboundedReceiver<UiAction>, front: &mut Frontend) {
    let net = Remote::new();
    let (ssid, networks) = net.scan();
    let wifi = Wifi::new(ssid, networks, WIFI_BAND.into());
    // The sender stays so the link never reports the core gone.
    let (_core_events, events) = mpsc::channel(1);
    if let Err(e) = serve(weak, actions, front, Link::Demo(net), events, System::demo(), wifi).await {
        tracing::error!("demo session failed: {e:#}");
    }
}
