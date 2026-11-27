//! Remote control: a small server inside r3cui so the remote can be watched and driven from the local network
//! (`crates/r3ctl` is the protocol, `r3ctl` a client). Switched on and off in Settings → System → Remote
//! control, with an access key shown there; a client proves it holds the key without sending it. While off nothing
//! listens; while on it waits on its socket and works only when asked, so a remote on battery pays only for use.

mod act;
pub(crate) mod server;

use crate::app::session::data_dir;
use crate::app::UiAction;
use crate::{AppWindow, RemoteControl};
use serde::{Deserialize, Serialize};
use slint::ComponentHandle;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, watch};

const FILE: &str = "remote-control.json";
/// Set to a key to switch remote control on with it, for a test install that a tool must reach at once.
const KEY_ENV: &str = "R3CUI_REMOTE_CONTROL_KEY";
/// Easy to read off the screen and type: no 0/O, 1/I/L.
const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
const KEY_LEN: usize = 10;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Config {
    enabled: bool,
    key: String,
}

fn load() -> Config {
    std::fs::read_to_string(data_dir().join(FILE)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save(config: &Config) {
    let path = data_dir().join(FILE);
    if let Err(e) = serde_json::to_string_pretty(config).map_err(std::io::Error::other).and_then(|t| std::fs::write(&path, t)) {
        tracing::warn!("remote control: saving {}: {e}", path.display());
    }
}

pub(crate) fn random_key() -> String {
    let mut bytes = [0u8; KEY_LEN];
    getrandom::getrandom(&mut bytes).expect("the system's random source");
    let chars: String = bytes.iter().map(|b| ALPHABET[usize::from(*b) % ALPHABET.len()] as char).collect();
    format!("{}-{}", &chars[..KEY_LEN / 2], &chars[KEY_LEN / 2..])
}

/// The switch and the key, as the settings page changes them.
#[derive(Clone)]
pub(crate) struct Handle {
    config: Arc<Mutex<Config>>,
    /// The key while it is on, `None` while off: what the server listens by.
    serving: watch::Sender<Option<String>>,
    weak: slint::Weak<AppWindow>,
    /// Kept off disk: a key from the environment is for one test run.
    from_env: bool,
}

impl Handle {
    pub(crate) fn toggle(&self) {
        let mut c = self.config.lock().unwrap();
        c.enabled = !c.enabled;
        if c.enabled && c.key.is_empty() {
            c.key = random_key();
        }
        self.apply(&c);
    }

    pub(crate) fn new_key(&self) {
        let mut c = self.config.lock().unwrap();
        c.key = random_key();
        self.apply(&c);
    }

    fn apply(&self, c: &Config) {
        if !self.from_env {
            save(c);
        }
        self.serving.send_replace(c.enabled.then(|| c.key.clone()));
        show(&self.weak, c);
    }
}

fn show(weak: &slint::Weak<AppWindow>, c: &Config) {
    let (on, key) = (c.enabled, c.key.clone());
    let address = server::address();
    let _ = weak.upgrade_in_event_loop(move |app| {
        let r = app.global::<RemoteControl>();
        r.set_on(on);
        r.set_key(key.as_str().into());
        r.set_address(address.as_str().into());
        let link = format!("r3ctl://{address}#{key}");
        r.set_qr(if on { crate::view::qr::image(&link, server::QR_SIZE).unwrap_or_default() } else { slint::Image::default() });
    });
}

/// Starts the server's thread; it listens only while switched on.
pub(crate) fn start(weak: slint::Weak<AppWindow>, actions: mpsc::UnboundedSender<UiAction>) -> Handle {
    let (config, from_env) = match std::env::var(KEY_ENV) {
        Ok(key) if !key.is_empty() => (Config { enabled: true, key }, true),
        _ => (load(), false),
    };
    let (serving, listen) = watch::channel(config.enabled.then(|| config.key.clone()));
    show(&weak, &config);
    let handle = Handle { config: Arc::new(Mutex::new(config)), serving, weak: weak.clone(), from_env };
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => return tracing::error!("remote control: {e}"),
        };
        rt.block_on(server::run(listen, weak, actions));
    });
    handle
}
