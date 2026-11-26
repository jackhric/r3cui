//! The demo driver: a receiver with four inputs, a film to play and a shelf of titles, all made up, so the real
//! frontend can be tried with nothing to connect to. Desktop builds only (the `demo` feature).

use crate::api::*;
use crate::config::{Binding, DriverEntry, Modules, Role, Slot};
use data::{COLUMNS, INPUTS, KODI, LEVEL_LIMIT, RECEIVER, SHELF, START_INPUT, START_LEVEL, SWITCH};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};

pub mod data;

pub static SPEC: Spec = Spec {
    id: "demo",
    name: "Demo",
    // A play mark in a rounded square.
    icon: "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2zm5 5v8l6-4z",
    fields: &[],
    provides: Provides { global: true, remote: true, ui: true },
};

struct State {
    volume: Volume,
    sources: Sources,
    power: Power,
    media: Option<Media>,
    shown: bool,
    focus: usize,
}

pub struct Demo {
    volume: Port<Option<Volume>, VolumeIntent>,
    source: Port<Option<Sources>, SourceIntent>,
    power: PowerPort,
    ui: UiPort,
    remote: mpsc::UnboundedSender<RemoteKey>,
    status: watch::Receiver<Status>,
}

/// A setup that points everything at the demo driver: global volume and source, every input's remote, and the screen of
/// the one input that plays (the others show their idle logo, as an input with nothing to show does).
pub fn modules() -> Modules {
    let mut m = Modules::default();
    let id = m.add_driver(SPEC.id, Default::default());
    let binding = || Some(Binding { driver: id.clone(), target: RECEIVER.into() });
    m.bind(Role::Volume, binding());
    m.bind(Role::Source, binding());
    m.bind(Role::Power, binding());
    for input in INPUTS {
        m.set_slot(input, Slot::Remote, Some(id.clone()));
    }
    m.set_slot(KODI, Slot::Ui, Some(id.clone()));
    m
}

impl Demo {
    pub fn start(_: &DriverEntry) -> Self {
        let state = Arc::new(Mutex::new(State {
            volume: Volume { level: START_LEVEL, max: LEVEL_LIMIT, muted: false },
            sources: Sources { current: START_INPUT.into(), list: INPUTS.map(String::from).to_vec() },
            power: Power { on: true },
            media: Some(data::film(Instant::now())),
            shown: false,
            focus: 0,
        }));
        let (volume_tx, mut volume_rx, volume) = port(None);
        let (source_tx, mut source_rx, source) = port(None);
        let (power_tx, mut power_rx, power) = port(None);
        let (stage_tx, mut ui_rx, ui) = port(Stage::default());
        let (remote, mut keys) = mpsc::unbounded_channel();
        let shelf = data::shelf();
        let publish = {
            let state = state.clone();
            move || {
                let s = state.lock().unwrap();
                volume_tx.send_replace(Some(s.volume.clone()));
                source_tx.send_replace(Some(s.sources.clone()));
                power_tx.send_replace(Some(s.power));
                let library = (s.shown && s.media.is_none()).then(|| Library { list: shelf.clone(), focus: s.focus });
                stage_tx.send_replace(Stage { media: s.media.clone(), library });
            }
        };
        publish();
        let (tx, mut changes) = mpsc::unbounded_channel::<Box<dyn FnOnce(&mut State) + Send>>();
        tokio::spawn({
            let state = state.clone();
            async move {
                while let Some(change) = changes.recv().await {
                    change(&mut state.lock().unwrap());
                    publish();
                }
            }
        });
        let change = move |f: Box<dyn FnOnce(&mut State) + Send>| drop(tx.send(f));
        tokio::spawn({
            let change = change.clone();
            async move {
                loop {
                    tokio::select! {
                        Some(i) = volume_rx.recv() => change(Box::new(move |s| {
                            let v = &mut s.volume;
                            match i {
                                VolumeIntent::Up => v.level = (v.level + 1).min(v.max),
                                VolumeIntent::Down => v.level = (v.level - 1).max(0),
                                VolumeIntent::Set(l) => v.level = l.clamp(0, v.max),
                                VolumeIntent::Mute(m) => v.muted = m,
                            }
                            if !matches!(i, VolumeIntent::Mute(_)) {
                                v.muted = false;
                            }
                        })),
                        Some(i) = source_rx.recv() => match i {
                            SourceIntent::Select(name) => {
                                let change = change.clone();
                                tokio::spawn(async move {
                                    tokio::time::sleep(SWITCH).await;
                                    change(Box::new(move |s| s.sources.current = name));
                                });
                            }
                        },
                        Some(PowerIntent::Set(on)) = power_rx.recv() => change(Box::new(move |s| s.power.on = on)),
                        Some(i) = ui_rx.recv() => change(Box::new(move |s| ui_intent(s, i))),
                        Some(k) = keys.recv() => change(Box::new(move |s| key(s, k))),
                        else => break,
                    }
                }
            }
        });
        Self { volume, source, power, ui, remote, status: watch::channel(Status::new(Health::Up)).1 }
    }
}

fn pause(s: &mut State) {
    if let Some(m) = &mut s.media {
        let now = Instant::now();
        (m.position, m.measured_at, m.paused) = (m.position_at(now), now, !m.paused);
    }
}

fn ui_intent(s: &mut State, i: UiIntent) {
    let now = Instant::now();
    match i {
        UiIntent::Shown(yes) => s.shown = yes,
        UiIntent::PlayPause => pause(s),
        UiIntent::Seek(by) => {
            if let Some(m) = &mut s.media {
                let at = m.position_at(now).as_secs_f64() + f64::from(by);
                (m.position, m.measured_at) = (Duration::from_secs_f64(at.clamp(0.0, m.total.as_secs_f64())), now);
            }
        }
        UiIntent::Audio(i) => {
            if let Some(m) = &mut s.media {
                m.audio_current = (i < m.audio.len()).then_some(i);
                m.format = m.audio_current.map(|i| m.audio[i].detail.clone()).unwrap_or_default();
            }
        }
        UiIntent::Subtitle(i) => {
            if let Some(m) = &mut s.media {
                m.subtitle_current = i.filter(|i| *i < m.subtitles.len());
            }
        }
    }
}

fn key(s: &mut State, k: RemoteKey) {
    let last = SHELF.len() - 1;
    match (k, s.media.is_some()) {
        (RemoteKey::PlayPause, true) => pause(s),
        (RemoteKey::Stop, _) => s.media = None,
        (RemoteKey::Select | RemoteKey::PlayPause, false) => s.media = Some(data::film(Instant::now())),
        (RemoteKey::Right, false) => s.focus = (s.focus + 1).min(last),
        (RemoteKey::Left, false) => s.focus = s.focus.saturating_sub(1),
        (RemoteKey::Down, false) => s.focus = (s.focus + COLUMNS).min(last),
        (RemoteKey::Up, false) => s.focus = s.focus.saturating_sub(COLUMNS),
        _ => {}
    }
}

impl Driver for Demo {
    fn spec(&self) -> &'static Spec {
        &SPEC
    }

    fn status(&self) -> watch::Receiver<Status> {
        self.status.clone()
    }

    fn volume(&self, _: &str) -> Option<VolumePort> {
        Some(self.volume.clone())
    }

    fn source(&self, _: &str) -> Option<SourcePort> {
        Some(self.source.clone())
    }

    fn power(&self, _: &str) -> Option<PowerPort> {
        Some(self.power.clone())
    }

    fn remote(&self) -> Option<mpsc::UnboundedSender<RemoteKey>> {
        Some(self.remote.clone())
    }

    fn ui(&self) -> Option<UiPort> {
        Some(self.ui.clone())
    }

    fn mark(&self, input: &str) -> &'static str {
        if input == KODI {
            crate::kodi::SPEC.icon
        } else {
            ""
        }
    }

    fn targets(&self, _: TargetKind) -> Targets<'_> {
        Box::pin(async { vec![Target { id: RECEIVER.into(), name: "Demo receiver".into() }] })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{entry, until};

    fn demo() -> Demo {
        Demo::start(&entry(&SPEC, &[]))
    }

    #[test]
    fn the_demo_setup_fills_the_slots() {
        let m = modules();
        assert!(m.global.volume.is_some() && m.global.source.is_some() && m.global.power.is_some());
        for input in INPUTS {
            let e = m.input(input).unwrap();
            assert_eq!((e.remote.as_deref(), e.ui.as_deref()), (Some(SPEC.id), (input == KODI).then_some(SPEC.id)), "{input}");
        }
    }

    #[tokio::test]
    async fn plays_switches_and_shows_its_shelf() {
        let d = demo();
        let (volume, source, ui) = (d.volume("x").unwrap(), d.source("x").unwrap(), d.ui().unwrap());
        assert_eq!(volume.get().unwrap().level, START_LEVEL);
        volume.send(VolumeIntent::Up);
        until(&mut volume.view.clone(), |v| v.as_ref().is_some_and(|v| v.level == START_LEVEL + 1)).await;
        source.send(SourceIntent::Select("KODI".into()));
        until(&mut source.view.clone(), |s| s.as_ref().is_some_and(|s| s.current == "KODI")).await;

        let power = d.power("x").unwrap();
        power.send(PowerIntent::Set(false));
        until(&mut power.view.clone(), |p| *p == Some(Power { on: false })).await;

        ui.send(UiIntent::Shown(true));
        d.remote().unwrap().send(RemoteKey::Stop).unwrap();
        let s = until(&mut ui.view.clone(), |s| s.library.is_some()).await;
        assert_eq!(s.library.unwrap().list.items.len(), SHELF.len());
        d.remote().unwrap().send(RemoteKey::Down).unwrap();
        until(&mut ui.view.clone(), |s| s.library.as_ref().is_some_and(|l| l.focus == COLUMNS)).await;
        d.remote().unwrap().send(RemoteKey::Select).unwrap();
        let s = until(&mut ui.view.clone(), |s| s.media.is_some()).await;
        assert_eq!(s.library, None);
        ui.send(UiIntent::Subtitle(Some(1)));
        until(&mut ui.view.clone(), |s| s.media.as_ref().is_some_and(|m| m.subtitle_current == Some(1))).await;
    }
}
