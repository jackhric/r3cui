//! `FakeDriver`: a driver with every port, for testing routing with no program behind it. Tests set what it shows
//! and read back what it was sent.

use r3cui_sdk::api::*;
use r3cui_sdk::DriverEntry;
use crate::registry::Registry;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, watch};

pub const FAKE_HOST: &str = "host";

pub static FAKE_GLOBAL: Spec = Spec { id: "fake-global", name: "Fake", icon: "", fields: &[], address: None, provides: Provides { global: true, remote: false, ui: false, power: false, voice: false }, pairing: None };
pub static FAKE_PLAYER: Spec = Spec {
    id: "fake-player",
    name: "Fake",
    icon: "",
    fields: &[Field { key: FAKE_HOST, label: "Host", kind: FieldKind::Text, required: true, default: None }],
    provides: Provides { global: false, remote: true, ui: true, power: false, voice: false },
    address: Some(FAKE_HOST),
    pairing: None,
};

pub const FAKE_PAIRABLE_CREDENTIAL: &str = "credential";
pub static FAKE_PAIRABLE: Spec = Spec {
    id: "fake-pairable",
    name: "Pairable",
    icon: "",
    fields: &[
        Field { key: FAKE_HOST, label: "Host", kind: FieldKind::Text, required: true, default: None },
        Field { key: FAKE_PAIRABLE_CREDENTIAL, label: "Credential", kind: FieldKind::Credential, required: false, default: None },
    ],
    provides: Provides { global: false, remote: true, ui: true, power: true, voice: false },
    address: None,
    pairing: Some(CodePairing { code_len: 6 }),
};

pub use r3cui_sdk::testing::{entry, until};

#[derive(Clone)]
pub struct FakeDriver(Arc<Inner>);

struct Inner {
    spec: &'static Spec,
    status: watch::Sender<Status>,
    volume: (watch::Sender<Option<Volume>>, Mutex<mpsc::UnboundedReceiver<VolumeIntent>>, VolumePort),
    source: (watch::Sender<Option<Sources>>, Mutex<mpsc::UnboundedReceiver<SourceIntent>>, SourcePort),
    power: (watch::Sender<Option<Power>>, Mutex<mpsc::UnboundedReceiver<PowerIntent>>, PowerPort),
    ui: (watch::Sender<Stage>, Mutex<mpsc::UnboundedReceiver<UiIntent>>, UiPort),
    keys: (mpsc::UnboundedSender<RemoteKey>, Mutex<mpsc::UnboundedReceiver<RemoteKey>>),
    bound: Mutex<Vec<String>>,
}

fn drain<T>(rx: &Mutex<mpsc::UnboundedReceiver<T>>) -> Vec<T> {
    let mut rx = rx.lock().unwrap();
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

impl FakeDriver {
    pub fn new(spec: &'static Spec) -> Self {
        let (v, vr, vp) = port(None);
        let (s, sr, sp) = port(None);
        let (pw, pwr, pwp) = port(None);
        let (u, ur, up) = port(Stage::default());
        let (ktx, krx) = mpsc::unbounded_channel();
        Self(Arc::new(Inner {
            spec,
            status: watch::channel(Status::new(Health::Connecting)).0,
            volume: (v, Mutex::new(vr), vp),
            source: (s, Mutex::new(sr), sp),
            power: (pw, Mutex::new(pwr), pwp),
            ui: (u, Mutex::new(ur), up),
            keys: (ktx, Mutex::new(krx)),
            bound: Mutex::default(),
        }))
    }

    pub fn set_status(&self, s: Status) {
        self.0.status.send_replace(s);
    }

    pub fn set_volume(&self, v: Volume) {
        self.0.volume.0.send_replace(Some(v));
    }

    pub fn set_sources(&self, s: Sources) {
        self.0.source.0.send_replace(Some(s));
    }

    pub fn set_power(&self, p: Power) {
        self.0.power.0.send_replace(Some(p));
    }

    pub fn set_stage(&self, s: Stage) {
        self.0.ui.0.send_replace(s);
    }

    pub fn keys(&self) -> Vec<RemoteKey> {
        drain(&self.0.keys.1)
    }

    pub fn volume_intents(&self) -> Vec<VolumeIntent> {
        drain(&self.0.volume.1)
    }

    pub fn source_intents(&self) -> Vec<SourceIntent> {
        drain(&self.0.source.1)
    }

    pub fn power_intents(&self) -> Vec<PowerIntent> {
        drain(&self.0.power.1)
    }

    pub fn ui_intents(&self) -> Vec<UiIntent> {
        drain(&self.0.ui.1)
    }

    /// The targets global control was bound to, in order.
    pub fn targets_bound(&self) -> Vec<String> {
        self.0.bound.lock().unwrap().clone()
    }
}

impl Driver for FakeDriver {
    fn spec(&self) -> &'static Spec {
        self.0.spec
    }

    fn status(&self) -> watch::Receiver<Status> {
        self.0.status.subscribe()
    }

    fn volume(&self, target: &str) -> Option<VolumePort> {
        self.0.bound.lock().unwrap().push(target.to_string());
        Some(self.0.volume.2.clone())
    }

    fn source(&self, target: &str) -> Option<SourcePort> {
        self.0.bound.lock().unwrap().push(target.to_string());
        Some(self.0.source.2.clone())
    }

    fn power(&self, target: &str) -> Option<PowerPort> {
        self.0.bound.lock().unwrap().push(target.to_string());
        Some(self.0.power.2.clone())
    }

    fn remote(&self) -> Option<mpsc::UnboundedSender<RemoteKey>> {
        Some(self.0.keys.0.clone())
    }

    fn ui(&self) -> Option<UiPort> {
        Some(self.0.ui.2.clone())
    }
}

/// The fakes a registry started, by driver type, and how often each was started.
#[derive(Clone, Default)]
pub struct Fakes(Arc<Mutex<HashMap<String, (FakeDriver, usize)>>>);

impl Fakes {
    pub fn registry(&self) -> Registry {
        let start = |fakes: Fakes| -> crate::registry::Start {
            Box::new(move |e: &DriverEntry| {
                let spec = match e.kind.as_str() {
                    k if k == FAKE_GLOBAL.id => &FAKE_GLOBAL,
                    k if k == FAKE_PAIRABLE.id => &FAKE_PAIRABLE,
                    _ => &FAKE_PLAYER,
                };
                let fake = FakeDriver::new(spec);
                let mut all = fakes.0.lock().unwrap();
                let starts = all.get(&e.kind).map_or(0, |(_, n)| *n);
                all.insert(e.kind.clone(), (fake.clone(), starts + 1));
                Arc::new(fake)
            })
        };
        Registry::new().with(&FAKE_GLOBAL, start(self.clone())).with(&FAKE_PLAYER, start(self.clone())).with(&FAKE_PAIRABLE, start(self.clone()))
    }

    pub fn get(&self, kind: &str) -> FakeDriver {
        self.0.lock().unwrap()[kind].0.clone()
    }

    pub fn started(&self, kind: &str) -> usize {
        self.0.lock().unwrap().get(kind).map_or(0, |(_, n)| *n)
    }
}
