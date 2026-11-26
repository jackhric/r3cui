//! Runs the drivers `modules.json` sets up and routes between them and the frontend: global control to its
//! bindings, keys and the stage to the current input's slots. The frontend sees ports and inputs, never a driver.

use crate::api::{Driver, KeyLight, RemoteKey, SourceIntent, SourcePort, Sources, Stage, Status, UiIntent, Volume, VolumeIntent, VolumePort};
use crate::config::{Binding, DriverEntry, Modules};
use crate::inputs::{self, Input};
use crate::registry::Registry;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::watch;
use tokio::task::JoinHandle;

struct Running {
    entry: DriverEntry,
    driver: Arc<dyn Driver>,
    /// Which start this is, so a port from before a restart is known to be stale.
    start: u64,
}

/// A global control port, with what it was bound to.
struct Bound<P> {
    port: P,
    start: u64,
    target: String,
}

pub struct Host {
    registry: Registry,
    modules: Modules,
    running: BTreeMap<String, Running>,
    volume: Option<Bound<VolumePort>>,
    source: Option<Bound<SourcePort>>,
    key_light: Option<watch::Receiver<Option<KeyLight>>>,
    /// Bumped whenever anything a driver shows changes.
    changed: watch::Sender<u64>,
    forwarders: Vec<JoinHandle<()>>,
    starts: u64,
    /// The UI slot told it is on screen, and whether it was told the remote is awake.
    shown: Option<(String, bool)>,
}

impl Host {
    /// Must be called within a tokio runtime: drivers start their tasks here.
    pub fn new(registry: Registry, modules: Modules) -> Self {
        let mut host = Self {
            registry,
            modules: Modules::default(),
            running: BTreeMap::new(),
            volume: None,
            source: None,
            key_light: None,
            changed: watch::channel(0).0,
            forwarders: Vec::new(),
            starts: 0,
            shown: None,
        };
        host.apply(modules);
        host
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn modules(&self) -> &Modules {
        &self.modules
    }

    /// Changes to the setup: drivers whose settings changed restart, removed ones stop, and every binding is made
    /// again.
    pub fn apply(&mut self, modules: Modules) {
        self.running.retain(|id, r| modules.drivers.get(id) == Some(&r.entry));
        for (id, entry) in &modules.drivers {
            if self.running.contains_key(id) {
                continue;
            }
            match self.registry.start(entry) {
                Some(driver) => {
                    tracing::info!("driver {id} ({}) started", entry.kind);
                    self.starts += 1;
                    self.running.insert(id.clone(), Running { entry: entry.clone(), driver, start: self.starts });
                }
                None => tracing::warn!("driver {id}: no driver type {:?}", entry.kind),
            }
        }
        if self.shown.as_ref().is_some_and(|(id, _)| !self.running.contains_key(id)) {
            self.shown = None;
        }
        self.modules = modules;
        self.bind();
    }

    fn bind(&mut self) {
        for f in self.forwarders.drain(..) {
            f.abort();
        }
        // A binding that hasn't changed keeps its port: a new one starts empty until the driver fills it, and the
        // inputs would blink out meanwhile.
        self.volume = rebind(self.volume.take(), &self.modules.global.volume, &self.running, |d, t| d.volume(t));
        self.source = rebind(self.source.take(), &self.modules.global.source, &self.running, |d, t| d.source(t));
        self.key_light = self.running.values().find_map(|r| r.driver.key_light());

        let changed = self.changed.clone();
        let mut watch_all = Vec::new();
        if let Some(b) = &self.volume {
            watch_all.push(forward(b.port.view.clone(), changed.clone()));
        }
        if let Some(b) = &self.source {
            watch_all.push(forward(b.port.view.clone(), changed.clone()));
        }
        if let Some(k) = &self.key_light {
            watch_all.push(forward(k.clone(), changed.clone()));
        }
        for r in self.running.values() {
            watch_all.push(forward(r.driver.status(), changed.clone()));
            if let Some(ui) = r.driver.ui() {
                watch_all.push(forward(ui.view, changed.clone()));
            }
        }
        self.forwarders = watch_all;
        self.changed.send_modify(|n| *n += 1);
    }

    /// Fires whenever there is something new to show.
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    pub fn driver(&self, id: &str) -> Option<Arc<dyn Driver>> {
        self.running.get(id).map(|r| r.driver.clone())
    }

    pub fn volume(&self) -> Option<Volume> {
        self.volume.as_ref().and_then(|b| b.port.get())
    }

    pub fn has_volume(&self) -> bool {
        self.volume.is_some()
    }

    pub fn send_volume(&self, intent: VolumeIntent) -> bool {
        self.volume.as_ref().map(|b| b.port.send(intent)).is_some()
    }

    pub fn sources(&self) -> Option<Sources> {
        self.source.as_ref().and_then(|b| b.port.get())
    }

    pub fn send_source(&self, intent: SourceIntent) -> bool {
        self.source.as_ref().map(|b| b.port.send(intent)).is_some()
    }

    pub fn key_light(&self) -> Option<KeyLight> {
        self.key_light.as_ref().and_then(|k| *k.borrow())
    }

    /// Every input, hidden ones included. Without a source binding, the one Default input; with one that hasn't
    /// reported yet, none.
    pub fn inputs(&self) -> Vec<Input> {
        match (&self.source, self.sources()) {
            (None, _) => inputs::inputs(None, &self.modules),
            (Some(_), Some(s)) => inputs::inputs(Some(&s.list), &self.modules),
            (Some(_), None) => Vec::new(),
        }
    }

    /// The input whose slots are in use while `source` is the one on (as the frontend shows it, which may be ahead
    /// of what the source has reported).
    pub fn current(&self, source: &str) -> Option<Input> {
        let all = self.inputs();
        let source = self.source.is_some().then_some(source);
        inputs::current(&all, source).cloned()
    }

    fn ui_of(&self, input: Option<&Input>) -> Option<crate::api::UiPort> {
        self.running.get(input?.ui.as_deref()?)?.driver.ui()
    }

    pub fn stage(&self, input: Option<&Input>) -> Stage {
        self.ui_of(input).map(|p| p.get()).unwrap_or_default()
    }

    /// `false` when the input has nothing in its remote slot.
    pub fn key(&self, input: Option<&Input>, key: RemoteKey) -> bool {
        let remote = input.and_then(|i| self.running.get(i.remote.as_deref()?)?.driver.remote());
        remote.is_some_and(|r| r.send(key).is_ok())
    }

    pub fn send_ui(&self, input: Option<&Input>, intent: UiIntent) -> bool {
        self.ui_of(input).map(|p| p.send(intent)).is_some()
    }

    /// Tells UI drivers whether they are on screen, as that changes.
    pub fn show(&mut self, input: Option<&Input>, awake: bool) {
        let now = input.and_then(|i| i.ui.clone()).filter(|id| self.running.contains_key(id)).map(|id| (id, awake));
        if now == self.shown {
            return;
        }
        if let Some((old, _)) = self.shown.take() {
            if now.as_ref().is_none_or(|(id, _)| *id != old) {
                if let Some(p) = self.running.get(&old).and_then(|r| r.driver.ui()) {
                    p.send(UiIntent::Shown(false));
                }
            }
        }
        if let Some((id, awake)) = &now {
            if let Some(p) = self.running.get(id).and_then(|r| r.driver.ui()) {
                p.send(UiIntent::Shown(*awake));
            }
        }
        self.shown = now;
    }

    /// One row per driver set up: its instance id, its name and how it is.
    pub fn statuses(&self) -> Vec<(String, String, Status)> {
        self.running.iter().map(|(id, r)| (id.clone(), r.driver.spec().name.to_string(), r.driver.status().borrow().clone())).collect()
    }

    /// The mark of the driver that fills `input`'s screen slot, or else its remote slot.
    pub fn icon(&self, input: &Input) -> Option<&'static str> {
        let mark = |slot: &Option<String>| Some(self.running.get(slot.as_deref()?)?.driver.mark(&input.id)).filter(|i| !i.is_empty());
        mark(&input.ui).or_else(|| mark(&input.remote))
    }

    /// Whether global control or `input` relies on the driver `id`.
    pub fn in_use(&self, id: &str, input: Option<&Input>) -> bool {
        let g = &self.modules.global;
        let bound = [&g.volume, &g.source].into_iter().flatten().any(|b| b.driver == id);
        bound || input.is_some_and(|i| i.remote.as_deref() == Some(id) || i.ui.as_deref() == Some(id))
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        for f in self.forwarders.drain(..) {
            f.abort();
        }
    }
}

fn rebind<P>(old: Option<Bound<P>>, binding: &Option<Binding>, running: &BTreeMap<String, Running>, make: impl Fn(&dyn Driver, &str) -> Option<P>) -> Option<Bound<P>> {
    let b = binding.as_ref()?;
    let r = running.get(&b.driver)?;
    if let Some(old) = old.filter(|o| o.start == r.start && o.target == b.target) {
        return Some(old);
    }
    Some(Bound { port: make(r.driver.as_ref(), &b.target)?, start: r.start, target: b.target.clone() })
}

fn forward<T: Send + Sync + 'static>(mut rx: watch::Receiver<T>, changed: watch::Sender<u64>) -> JoinHandle<()> {
    tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            changed.send_modify(|n| *n += 1);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{Health, Media};
    use crate::config::{Role, Slot};
    use crate::inputs::DEFAULT_INPUT;
    use crate::testing::{FakeDriver, Fakes, FAKE_GLOBAL, FAKE_HOST, FAKE_PLAYER};
    use std::time::{Duration, Instant};

    fn setup() -> (Host, Fakes) {
        let fakes = Fakes::default();
        let mut m = Modules::default();
        m.add_driver(FAKE_GLOBAL.id, Default::default());
        m.add_driver(FAKE_PLAYER.id, Default::default());
        m.bind(Role::Volume, Some(Binding { driver: FAKE_GLOBAL.id.into(), target: "receiver".into() }));
        m.bind(Role::Source, Some(Binding { driver: FAKE_GLOBAL.id.into(), target: "receiver".into() }));
        m.set_slot("KODI", Slot::Remote, Some(FAKE_PLAYER.id.into()));
        m.set_slot("KODI", Slot::Ui, Some(FAKE_PLAYER.id.into()));
        (Host::new(fakes.registry(), m), fakes)
    }

    fn report_sources(fake: &FakeDriver, current: &str) {
        fake.set_sources(Sources { on: true, current: current.into(), list: vec!["TV".into(), "KODI".into()] });
    }

    #[tokio::test]
    async fn keys_go_to_the_current_inputs_remote_slot() {
        let (host, fakes) = setup();
        let (global, player) = (fakes.get(FAKE_GLOBAL.id), fakes.get(FAKE_PLAYER.id));
        assert!(host.inputs().is_empty(), "the source hasn't reported yet");
        report_sources(&global, "KODI");
        let kodi = host.current("KODI");
        assert!(host.key(kodi.as_ref(), RemoteKey::Up));
        assert!(!host.key(host.current("TV").as_ref(), RemoteKey::Down), "TV has no remote slot");
        tokio::task::yield_now().await;
        assert_eq!(player.keys(), [RemoteKey::Up]);
        assert!(global.keys().is_empty());
    }

    #[tokio::test]
    async fn volume_and_source_go_to_their_bindings() {
        let (host, fakes) = setup();
        let global = fakes.get(FAKE_GLOBAL.id);
        global.set_volume(Volume { level: 40, max: 72, muted: false });
        assert_eq!(host.volume().map(|v| v.level), Some(40));
        assert!(host.send_volume(VolumeIntent::Up));
        assert!(host.send_source(SourceIntent::Select("TV".into())));
        tokio::task::yield_now().await;
        assert_eq!(global.volume_intents(), [VolumeIntent::Up]);
        assert_eq!(global.source_intents(), [SourceIntent::Select("TV".into())]);
        assert_eq!(global.targets_bound(), ["receiver", "receiver"]);
    }

    #[tokio::test]
    async fn the_stage_follows_the_input_and_ui_drivers_hear_when_shown() {
        let (mut host, fakes) = setup();
        let (global, player) = (fakes.get(FAKE_GLOBAL.id), fakes.get(FAKE_PLAYER.id));
        report_sources(&global, "KODI");
        let media = Media {
            title: "2001".into(),
            subtitle: String::new(),
            paused: false,
            position: Duration::ZERO,
            measured_at: Instant::now(),
            total: Duration::from_secs(9000),
            format: String::new(),
            poster: String::new(),
            backdrop: String::new(),
            audio: vec![],
            audio_current: None,
            subtitles: vec![],
            subtitle_current: None,
        };
        player.set_stage(Stage { media: Some(media), library: None });
        let kodi = host.current("KODI");
        assert_eq!(host.stage(kodi.as_ref()).media.unwrap().title, "2001");
        assert_eq!(host.stage(host.current("TV").as_ref()), Stage::default(), "an empty slot is a blank stage");
        host.show(kodi.as_ref(), true);
        host.show(kodi.as_ref(), true);
        host.show(host.current("TV").as_ref(), true);
        tokio::task::yield_now().await;
        assert_eq!(player.ui_intents(), [UiIntent::Shown(true), UiIntent::Shown(false)]);
    }

    #[tokio::test]
    async fn changes_wake_the_frontend() {
        let (host, fakes) = setup();
        let mut changes = host.changes();
        changes.borrow_and_update();
        fakes.get(FAKE_PLAYER.id).set_status(Status::new(Health::Up));
        tokio::time::timeout(Duration::from_secs(1), changes.changed()).await.unwrap().unwrap();
        assert_eq!(host.statuses().iter().map(|(id, _, s)| (id.as_str(), s.health)).collect::<Vec<_>>(), [(FAKE_GLOBAL.id, Health::Connecting), (FAKE_PLAYER.id, Health::Up)]);
        assert!(host.in_use(FAKE_GLOBAL.id, None));
        report_sources(&fakes.get(FAKE_GLOBAL.id), "KODI");
        assert!(host.in_use(FAKE_PLAYER.id, host.current("KODI").as_ref()) && !host.in_use(FAKE_PLAYER.id, None));
    }

    #[tokio::test]
    async fn without_a_source_the_default_input_has_the_slots() {
        let fakes = Fakes::default();
        let mut m = Modules::default();
        m.add_driver(FAKE_PLAYER.id, Default::default());
        m.set_slot(DEFAULT_INPUT, Slot::Remote, Some(FAKE_PLAYER.id.into()));
        let host = Host::new(fakes.registry(), m);
        assert!(host.key(host.current("anything").as_ref(), RemoteKey::Select));
        assert_eq!(host.volume(), None);
        assert!(!host.send_volume(VolumeIntent::Up));
    }

    #[tokio::test]
    async fn other_changes_keep_global_control_bound() {
        let (mut host, fakes) = setup();
        report_sources(&fakes.get(FAKE_GLOBAL.id), "KODI");
        let mut m = host.modules().clone();
        m.set_hidden("TV", true);
        host.apply(m);
        assert_eq!(fakes.get(FAKE_GLOBAL.id).targets_bound().len(), 2, "not asked for new ports");
        assert_eq!(host.inputs().len(), 2, "the inputs never blink out");
    }

    #[tokio::test]
    async fn removing_a_driver_stops_it_and_unbinds() {
        let (mut host, fakes) = setup();
        let mut m = host.modules().clone();
        m.remove_driver(FAKE_GLOBAL.id);
        host.apply(m);
        assert!(!host.send_volume(VolumeIntent::Up));
        assert_eq!(fakes.started(FAKE_PLAYER.id), 1, "an unchanged driver keeps running");
        let mut m = host.modules().clone();
        m.set_field(FAKE_PLAYER.id, FAKE_HOST, "elsewhere");
        host.apply(m);
        assert_eq!(fakes.started(FAKE_PLAYER.id), 2, "a changed one restarts");
    }
}
