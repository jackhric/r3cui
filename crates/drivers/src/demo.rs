//! The demo driver: a receiver with four inputs, a film to play and a shelf of titles, all made up, so the real
//! frontend can be tried with nothing to connect to. Desktop builds only (the `demo` feature).

use crate::api::*;
use crate::config::DriverEntry;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};

pub static SPEC: Spec = Spec { id: "demo", name: "Demo", fields: &[], provides: Provides { global: true, remote: true, ui: true } };

const INPUTS: [&str; 4] = ["ANDROID TV", "KODI", "PLAYSTATION 5", "XBOX 360"];
const FILM: (&str, u64) = ("Mad Max: Fury Road", 7200);
const SHELF: [(&str, &str); 6] = [
    ("2001: A Space Odyssey", "1968 · 2h 29m"),
    ("Alien", "1979 · 1h 57m"),
    ("Arrival", "2016 · 1h 56m"),
    ("Blade Runner", "1982 · 1h 57m"),
    ("Mad Max: Fury Road", "2015 · 2h 0m"),
    ("The Godfather", "1972 · 2h 55m"),
];
/// How long the receiver takes to switch input, so the screen's "switching" shows.
const SWITCH: Duration = Duration::from_millis(1500);

struct State {
    volume: Volume,
    sources: Sources,
    media: Option<Media>,
    shown: bool,
    focus: usize,
}

pub struct Demo {
    volume: Port<Option<Volume>, VolumeIntent>,
    source: Port<Option<Sources>, SourceIntent>,
    ui: UiPort,
    remote: mpsc::UnboundedSender<RemoteKey>,
    status: watch::Receiver<Status>,
}

fn film(now: Instant) -> Media {
    let track = |label: &str, detail: &str| Track { label: label.into(), detail: detail.into() };
    Media {
        title: FILM.0.into(),
        subtitle: String::new(),
        paused: false,
        position: Duration::from_secs(2788),
        measured_at: now,
        total: Duration::from_secs(FILM.1),
        format: "DTS-HD MA 7.1".into(),
        poster: String::new(),
        backdrop: String::new(),
        audio: vec![track("English", "DTS-HD MA 7.1"), track("English Commentary", "AAC stereo")],
        audio_current: Some(0),
        subtitles: vec![track("English", ""), track("English SDH", ""), track("French · Forced", "")],
        subtitle_current: None,
    }
}

fn shelf() -> Arc<LibraryList> {
    let items = SHELF
        .iter()
        .map(|(title, meta)| LibraryItem {
            title: (*title).into(),
            meta: (*meta).into(),
            rating: "8.1".into(),
            tail: "R".into(),
            genre: "Science Fiction".into(),
            plot: String::new(),
            detail: String::new(),
            progress: 0.0,
            poster: String::new(),
        })
        .collect();
    Arc::new(LibraryList { id: u64::MAX, heading: "MOVIES".into(), count: format!("{} films", SHELF.len()), items })
}

impl Demo {
    pub fn start(_: &DriverEntry) -> Self {
        let state = Arc::new(Mutex::new(State {
            volume: Volume { level: 62, max: 72, muted: false },
            sources: Sources { on: true, current: "PLAYSTATION 5".into(), list: INPUTS.map(String::from).to_vec() },
            media: Some(film(Instant::now())),
            shown: false,
            focus: 0,
        }));
        let (volume_tx, mut volume_rx, volume) = port(None);
        let (source_tx, mut source_rx, source) = port(None);
        let (stage_tx, mut ui_rx, ui) = port(Stage::default());
        let (remote, mut keys) = mpsc::unbounded_channel();
        let shelf = shelf();
        let publish = {
            let state = state.clone();
            move || {
                let s = state.lock().unwrap();
                volume_tx.send_replace(Some(s.volume.clone()));
                source_tx.send_replace(Some(s.sources.clone()));
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
                            SourceIntent::Power(on) => change(Box::new(move |s| s.sources.on = on)),
                        },
                        Some(i) = ui_rx.recv() => change(Box::new(move |s| ui_intent(s, i))),
                        Some(k) = keys.recv() => change(Box::new(move |s| key(s, k))),
                        else => break,
                    }
                }
            }
        });
        Self { volume, source, ui, remote, status: watch::channel(Status::new(Health::Up)).1 }
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

/// The shelf is three to a row.
fn key(s: &mut State, k: RemoteKey) {
    let last = SHELF.len() - 1;
    match (k, s.media.is_some()) {
        (RemoteKey::PlayPause, true) => pause(s),
        (RemoteKey::Stop, _) => s.media = None,
        (RemoteKey::Select | RemoteKey::PlayPause, false) => s.media = Some(film(Instant::now())),
        (RemoteKey::Right, false) => s.focus = (s.focus + 1).min(last),
        (RemoteKey::Left, false) => s.focus = s.focus.saturating_sub(1),
        (RemoteKey::Down, false) => s.focus = (s.focus + 3).min(last),
        (RemoteKey::Up, false) => s.focus = s.focus.saturating_sub(3),
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

    fn remote(&self) -> Option<mpsc::UnboundedSender<RemoteKey>> {
        Some(self.remote.clone())
    }

    fn ui(&self) -> Option<UiPort> {
        Some(self.ui.clone())
    }

    fn targets(&self, _: TargetKind) -> Targets<'_> {
        Box::pin(async { vec![Target { id: "demo.receiver".into(), name: "Demo receiver".into() }] })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Modules;

    async fn until<T: Clone>(rx: &mut watch::Receiver<T>, ok: impl Fn(&T) -> bool) -> T {
        tokio::time::timeout(Duration::from_secs(5), rx.wait_for(|v| ok(v))).await.expect("in time").expect("open").clone()
    }

    fn demo() -> Demo {
        let mut m = Modules::default();
        m.add_driver("demo", Default::default());
        Demo::start(&m.drivers["demo"])
    }

    #[tokio::test]
    async fn plays_switches_and_shows_its_shelf() {
        let d = demo();
        let (volume, source, ui) = (d.volume("x").unwrap(), d.source("x").unwrap(), d.ui().unwrap());
        assert_eq!(volume.get().unwrap().level, 62);
        volume.send(VolumeIntent::Up);
        until(&mut volume.view.clone(), |v| v.as_ref().is_some_and(|v| v.level == 63)).await;
        source.send(SourceIntent::Select("KODI".into()));
        until(&mut source.view.clone(), |s| s.as_ref().is_some_and(|s| s.current == "KODI")).await;

        ui.send(UiIntent::Shown(true));
        d.remote().unwrap().send(RemoteKey::Stop).unwrap();
        let s = until(&mut ui.view.clone(), |s| s.library.is_some()).await;
        assert_eq!(s.library.unwrap().list.items.len(), SHELF.len());
        d.remote().unwrap().send(RemoteKey::Down).unwrap();
        until(&mut ui.view.clone(), |s| s.library.as_ref().is_some_and(|l| l.focus == 3)).await;
        d.remote().unwrap().send(RemoteKey::Select).unwrap();
        let s = until(&mut ui.view.clone(), |s| s.media.is_some()).await;
        assert_eq!(s.library, None);
        ui.send(UiIntent::Subtitle(Some(1)));
        until(&mut ui.view.clone(), |s| s.media.as_ref().is_some_and(|m| m.subtitle_current == Some(1))).await;
    }
}
