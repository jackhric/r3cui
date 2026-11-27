//! r3cui's own web configurator (`docs/web-configurator.md`): a page served from the remote, on the local network, to
//! set up integrations, global control and inputs from a computer. Switched on from Settings → Integrations → Set up
//! from a computer, with a PIN shown there. Off unless switched on, and off again after a while unused; never on at
//! start-up, so a forgotten configurator does not outlive a restart.

mod server;

use crate::app::UiAction;
use crate::remote_control::server::lan_ip;
use crate::{AppWindow, R3Web};
use slint::ComponentHandle;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

pub(crate) const PORT: u16 = 10781;
/// Unused this long, it switches itself off.
const IDLE: Duration = Duration::from_secs(30 * 60);
const PIN_LEN: usize = 4;

/// The switch and the PIN, as the remote's page changes them.
#[derive(Clone)]
pub(crate) struct Handle {
    /// The PIN while it is on, `None` while off: what the server listens by.
    serving: watch::Sender<Option<String>>,
    weak: slint::Weak<AppWindow>,
}

impl Handle {
    /// On with a fresh PIN, unless it is on already.
    pub(crate) fn open(&self) {
        if self.serving.borrow().is_none() {
            self.new_pin();
        }
    }

    /// A new PIN signs out every browser signed in with the old one.
    pub(crate) fn new_pin(&self) {
        self.set(Some(random_pin()));
    }

    pub(crate) fn off(&self) {
        self.set(None);
    }

    fn set(&self, pin: Option<String>) {
        self.serving.send_replace(pin.clone());
        show(&self.weak, pin);
    }
}

fn random_pin() -> String {
    let mut bytes = [0u8; PIN_LEN];
    getrandom::getrandom(&mut bytes).expect("the system's random source");
    bytes.iter().map(|b| char::from(b'0' + b % 10)).collect()
}

fn show(weak: &slint::Weak<AppWindow>, pin: Option<String>) {
    let address = format!("{}:{PORT}", lan_ip());
    let _ = weak.upgrade_in_event_loop(move |app| {
        let w = app.global::<R3Web>();
        w.set_on(pin.is_some());
        let digits: Vec<slint::SharedString> = pin.iter().flat_map(|p| p.chars()).map(|c| c.to_string().into()).collect();
        w.set_pin(std::rc::Rc::new(slint::VecModel::from(digits)).into());
        w.set_address(address.as_str().into());
        let link = format!("http://{address}/");
        w.set_qr(if pin.is_some() { crate::view::qr::image(&link, crate::remote_control::server::QR_SIZE).unwrap_or_default() } else { slint::Image::default() });
    });
}

static HANDLE: OnceLock<Handle> = OnceLock::new();

/// The switch, once `start` has run.
pub(crate) fn handle() -> Option<&'static Handle> {
    HANDLE.get()
}

/// Starts the server's thread; it listens only while switched on.
pub(crate) fn start(weak: slint::Weak<AppWindow>, actions: mpsc::UnboundedSender<UiAction>) -> Handle {
    let (serving, listen) = watch::channel(None);
    show(&weak, None);
    let handle = Handle { serving, weak };
    let _ = HANDLE.set(handle.clone());
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => return tracing::error!("web configurator: {e}"),
        };
        rt.block_on(server::run(listen, actions));
    });
    handle
}
