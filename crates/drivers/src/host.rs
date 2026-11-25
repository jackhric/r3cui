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
}

pub struct Host {
    registry: Registry,
    modules: Modules,
    running: BTreeMap<String, Running>,
    volume: Option<VolumePort>,
    source: Option<SourcePort>,
    key_light: Option<watch::Receiver<Option<KeyLight>>>,
    /// Bumped whenever anything a driver shows changes.
    changed: watch::Sender<u64>,
    forwarders: Vec<JoinHandle<()>>,
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
                    self.running.insert(id.clone(), Running { entry: entry.clone(), driver });
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
        let find = |b: &Option<Binding>| b.as_ref().and_then(|b| Some((self.running.get(&b.driver)?.driver.clone(), b.target.clone())));
        self.volume = find(&self.modules.global.volume).and_then(|(d, t)| d.volume(&t));
        self.source = find(&self.modules.global.source).and_then(|(d, t)| d.source(&t));
        self.key_light = self.running.values().find_map(|r| r.driver.key_light());

        let changed = self.changed.clone();
        let mut watch_all = Vec::new();
        if let Some(p) = &self.volume {
            watch_all.push(forward(p.view.clone(), changed.clone()));
        }
        if let Some(p) = &self.source {
            watch_all.push(forward(p.view.clone(), changed.clone()));
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
        self.volume.as_ref().and_then(|p| p.get())
    }

    pub fn has_volume(&self) -> bool {
        self.volume.is_some()
    }

    pub fn send_volume(&self, intent: VolumeIntent) -> bool {
        self.volume.as_ref().map(|p| p.send(intent)).is_some()
    }

    pub fn sources(&self) -> Option<Sources> {
        self.source.as_ref().and_then(|p| p.get())
    }

    pub fn send_source(&self, intent: SourceIntent) -> bool {
        self.source.as_ref().map(|p| p.send(intent)).is_some()
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
    use crate::testing::{FakeDriver, Fakes};
    use std::time::{Duration, Instant};

    fn setup() -> (Host, Fakes) {
        let fakes = Fakes::default();
        let mut m = Modules::default();
        m.add_driver("fake-global", Default::default());
        m.add_driver("fake-player", Default::default());
        m.bind(Role::Volume, Some(Binding { driver: "fake-global".into(), target: "receiver".into() }));
        m.bind(Role::Source, Some(Binding { driver: "fake-global".into(), target: "receiver".into() }));
        m.set_slot("KODI", Slot::Remote, Some("fake-player".into()));
        m.set_slot("KODI", Slot::Ui, Some("fake-player".into()));
        (Host::new(fakes.registry(), m), fakes)
    }

    fn report_sources(fake: &FakeDriver, current: &str) {
        fake.set_sources(Sources { on: true, current: current.into(), list: vec!["TV".into(), "KODI".into()] });
    }

    #[tokio::test]
    async fn keys_go_to_the_current_inputs_remote_slot() {
        let (host, fakes) = setup();
        let (global, player) = (fakes.get("fake-global"), fakes.get("fake-player"));
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
        let global = fakes.get("fake-global");
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
        let (global, player) = (fakes.get("fake-global"), fakes.get("fake-player"));
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
        fakes.get("fake-player").set_status(Status::new(Health::Up));
        tokio::time::timeout(Duration::from_secs(1), changes.changed()).await.unwrap().unwrap();
        assert_eq!(host.statuses().iter().map(|(id, _, s)| (id.as_str(), s.health)).collect::<Vec<_>>(), [("fake-global", Health::Connecting), ("fake-player", Health::Up)]);
        assert!(host.in_use("fake-global", None));
        report_sources(&fakes.get("fake-global"), "KODI");
        assert!(host.in_use("fake-player", host.current("KODI").as_ref()) && !host.in_use("fake-player", None));
    }

    #[tokio::test]
    async fn without_a_source_the_default_input_has_the_slots() {
        let fakes = Fakes::default();
        let mut m = Modules::default();
        m.add_driver("fake-player", Default::default());
        m.set_slot(DEFAULT_INPUT, Slot::Remote, Some("fake-player".into()));
        let host = Host::new(fakes.registry(), m);
        assert!(host.key(host.current("anything").as_ref(), RemoteKey::Select));
        assert_eq!(host.volume(), None);
        assert!(!host.send_volume(VolumeIntent::Up));
    }

    #[tokio::test]
    async fn removing_a_driver_stops_it_and_unbinds() {
        let (mut host, fakes) = setup();
        let mut m = host.modules().clone();
        m.remove_driver("fake-global");
        host.apply(m);
        assert!(!host.send_volume(VolumeIntent::Up));
        assert_eq!(fakes.started("fake-player"), 1, "an unchanged driver keeps running");
        let mut m = host.modules().clone();
        m.set_field("fake-player", "host", "elsewhere");
        host.apply(m);
        assert_eq!(fakes.started("fake-player"), 2, "a changed one restarts");
    }
}
