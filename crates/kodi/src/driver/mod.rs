//! The Kodi driver, on r3cui's own client (`crates/kodi`): Kodi's keys in an input's remote slot, and Now Playing
//! or the library wall in its UI slot.

mod cursor;
mod entry;
mod library;

pub use entry::entry as library_item;

use r3cui_sdk::api::*;
use r3cui_sdk::DriverEntry;
use r3cui_sdk::run::follow;
use crate::ids::{AudioStreamIndex, SubtitleIndex};
use crate::labels::{audio_format, audio_label, subtitle_label};
use crate::methods::input::InputAction;
use crate::{KodiConfig, KodiHandle, KodiService, KodiState, Link, Nav, SubtitleChoice};
use library::Wall;
use std::time::Instant;
use tokio::sync::{mpsc, watch};

pub mod field {
    pub const HOST: &str = "host";
    pub const WS_PORT: &str = "ws_port";
    pub const HTTP_PORT: &str = "http_port";
}

pub static SPEC: Spec = Spec {
    id: "kodi",
    name: "Kodi",
    // Simple Icons (CC0); the mark is Kodi's.
    icon: "M12.03.047c-.226 0-.452.107-.669.324-.922.922-1.842 1.845-2.763 2.768-.233.233-.455.48-.703.695-.31.267-.405.583-.399.988.02 1.399.008 2.799.008 4.198 0 1.453-.002 2.907 0 4.36 0 .11.002.223.03.327.087.337.303.393.546.15 1.31-1.31 2.618-2.622 3.928-3.933l4.449-4.453c.43-.431.43-.905 0-1.336L12.697.37c-.216-.217-.442-.324-.668-.324zm7.224 7.23c-.223 0-.445.104-.65.309L14.82 11.37c-.428.429-.427.895 0 1.322l3.76 3.766c.44.44.908.44 1.346.002 1.215-1.216 2.427-2.433 3.644-3.647.182-.18.353-.364.43-.615v-.33c-.077-.251-.246-.436-.428-.617-1.224-1.22-2.443-2.445-3.666-3.668-.205-.205-.429-.307-.652-.307zM4.18 7.611c-.086.014-.145.094-.207.157L.209 11.572c-.28.284-.278.677.004.96l2.043 2.046c.59.59 1.177 1.182 1.767 1.772.169.168.33.139.416-.084.044-.114.062-.242.063-.364.004-1.283.004-2.567.004-3.851h-.002V8.184c0-.085-.01-.169-.022-.252-.019-.135-.072-.258-.207-.309a.186.186 0 0 0-.095-.012zm7.908 6.838c-.224 0-.447.106-.656.315L7.66 18.537c-.433.434-.433.899.002 1.334 1.215 1.216 2.43 2.43 3.643 3.649.18.18.361.354.611.433h.33c.244-.069.423-.226.598-.402 1.222-1.23 2.45-2.453 3.676-3.68.43-.43.427-.905-.004-1.338l-3.772-3.773c-.208-.208-.432-.311-.656-.31z",
    fields: &[
        Field { key: field::HOST, label: "Address", kind: FieldKind::Text, required: true, default: None },
        Field { key: field::WS_PORT, label: "WebSocket port", kind: FieldKind::Port, required: false, default: Some("9090") },
        Field { key: field::HTTP_PORT, label: "Web server port", kind: FieldKind::Port, required: false, default: Some("8080") },
    ],
    provides: Provides { global: false, remote: true, ui: true, power: false, voice: false },
    address: Some(field::HOST),
    pairing: None,
};

/// What the app lists to carry this integration.
pub const INTEGRATION: r3cui_sdk::Integration = r3cui_sdk::Integration { spec: &SPEC, start: |e| std::sync::Arc::new(Kodi::start(e)) };

pub struct Kodi {
    status: watch::Receiver<Status>,
    remote: mpsc::UnboundedSender<RemoteKey>,
    ui: UiPort,
}

impl Kodi {
    pub fn start(entry: &DriverEntry) -> Self {
        let (remote, mut keys) = mpsc::unbounded_channel();
        let (stage, mut intents, ui) = port(Stage::default());
        let Some(host) = entry.text(field::HOST) else {
            return Self { status: watch::channel(Status::with(Health::NotSetUp, "Address missing")).1, remote, ui };
        };
        let mut cfg = KodiConfig::new(host);
        let port = |key| entry.text(key).and_then(|p| p.parse().ok());
        if let Some(p) = port(field::WS_PORT) {
            cfg.ws_port = p;
        }
        if let Some(p) = port(field::HTTP_PORT) {
            cfg.http_port = p;
        }
        let kodi = KodiService::start(cfg);

        let (status_tx, status) = watch::channel(Status::new(Health::Connecting));
        follow(kodi.state(), status_tx, |s: &KodiState| status_of(&s.link));

        let (wall, mut library) = Wall::start(kodi.clone());
        tokio::spawn({
            let (kodi, mut state) = (kodi.clone(), kodi.state());
            async move {
                loop {
                    let media = media_of(&kodi, &state.borrow_and_update());
                    let shelf = if media.is_none() { library.borrow_and_update().clone() } else { None };
                    stage.send_replace(Stage { media, library: shelf, app: None });
                    tokio::select! {
                        r = state.changed() => if r.is_err() { break },
                        r = library.changed() => if r.is_err() { break },
                        _ = stage.closed() => break,
                    }
                }
            }
        });
        tokio::spawn({
            let (kodi, wall) = (kodi.clone(), wall.clone());
            async move {
                while let Some(key) = keys.recv().await {
                    wall.pressed(key);
                    press(&kodi, key);
                }
            }
        });
        tokio::spawn(async move {
            while let Some(intent) = intents.recv().await {
                match intent {
                    UiIntent::Shown(yes) => wall.shown(yes),
                    UiIntent::PlayPause => kodi.play_pause(),
                    UiIntent::Seek(by) => spawn_logged("seek", { let k = kodi.clone(); async move { k.seek_by(by).await } }),
                    UiIntent::Audio(i) => pick_audio(&kodi, i),
                    UiIntent::Subtitle(i) => pick_subtitle(&kodi, i),
                }
            }
        });
        Self { status, remote, ui }
    }
}

impl Driver for Kodi {
    fn spec(&self) -> &'static Spec {
        &SPEC
    }

    fn status(&self) -> watch::Receiver<Status> {
        self.status.clone()
    }

    fn remote(&self) -> Option<mpsc::UnboundedSender<RemoteKey>> {
        Some(self.remote.clone())
    }

    fn ui(&self) -> Option<UiPort> {
        Some(self.ui.clone())
    }
}

fn status_of(link: &Link) -> Status {
    match link {
        Link::Unconfigured => Status::new(Health::NotSetUp),
        Link::Connecting { .. } => Status::new(Health::Connecting),
        Link::Up => Status::new(Health::Up),
        Link::Down { .. } => Status::new(Health::Down),
    }
}

fn media_of(kodi: &KodiHandle, state: &KodiState) -> Option<Media> {
    let p = state.playback.as_ref()?;
    let audio = p.audio_current.and_then(|i| p.audio.iter().find(|a| a.index == i));
    let a = &p.item.art;
    let url = |r: Option<&crate::wire::ArtRef>| r.map(|r| kodi.config().image_url(r)).unwrap_or_default();
    Some(Media {
        title: p.item.title.clone(),
        subtitle: p.item.show.as_ref().map(|s| format!("Season {} · Episode {}", s.season, s.episode)).unwrap_or_default(),
        paused: p.timeline.speed.is_paused(),
        position: p.timeline.position_at(Instant::now()),
        measured_at: Instant::now(),
        total: p.timeline.total,
        format: audio.map(audio_format).unwrap_or_default(),
        poster: url(a.poster.as_ref().or(a.show_poster.as_ref()).or(a.thumb.as_ref())),
        backdrop: url(a.fanart.as_ref().or(a.show_fanart.as_ref())),
        audio: p.audio.iter().map(|a| Track { label: audio_label(a), detail: audio_format(a) }).collect(),
        audio_current: p.audio.iter().position(|a| Some(a.index) == p.audio_current),
        subtitles: p.subtitles.iter().map(|s| Track { label: subtitle_label(s), detail: String::new() }).collect(),
        subtitle_current: p.subtitle.and_then(|i| p.subtitles.iter().position(|s| s.index == i)),
    })
}

/// Kodi's own keys where it has them. Menu is the context menu while browsing and the on-screen display while
/// playing; the channel keys page through lists and skip chapters.
fn press(kodi: &KodiHandle, key: RemoteKey) {
    let playing = kodi.state().borrow().playback.is_some();
    match key {
        RemoteKey::Up => kodi.nav(Nav::Up),
        RemoteKey::Down => kodi.nav(Nav::Down),
        RemoteKey::Left => kodi.nav(Nav::Left),
        RemoteKey::Right => kodi.nav(Nav::Right),
        RemoteKey::Select => kodi.nav(Nav::Select),
        RemoteKey::Back => kodi.nav(Nav::Back),
        RemoteKey::Menu => kodi.nav(if playing { Nav::Osd } else { Nav::ContextMenu }),
        RemoteKey::ChannelUp => kodi.action(if playing { InputAction::ChapterOrBigStepForward } else { InputAction::PageUp }),
        RemoteKey::ChannelDown => kodi.action(if playing { InputAction::ChapterOrBigStepBack } else { InputAction::PageDown }),
        RemoteKey::Previous => kodi.action(InputAction::SkipPrevious),
        RemoteKey::Next => kodi.action(InputAction::SkipNext),
        RemoteKey::PlayPause => kodi.play_pause(),
        RemoteKey::Stop => kodi.stop(),
        RemoteKey::Record => {}
    }
}

fn spawn_logged<F>(what: &'static str, f: F)
where
    F: std::future::Future<Output = crate::Result<()>> + Send + 'static,
{
    tokio::spawn(async move {
        if let Err(e) = f.await {
            tracing::warn!("kodi {what}: {e}");
        }
    });
}

fn pick_audio(kodi: &KodiHandle, row: usize) {
    let stream = kodi.state().borrow().playback.as_ref().and_then(|p| p.audio.get(row).map(|a| a.index.0));
    if let Some(i) = stream {
        let kodi = kodi.clone();
        spawn_logged("audio", async move { kodi.set_audio(AudioStreamIndex(i)).await });
    }
}

/// With a reload, so bitmap subtitles show from the next line rather than after Kodi's read-ahead runs out.
fn pick_subtitle(kodi: &KodiHandle, row: Option<usize>) {
    let choice = match row {
        None => Some(SubtitleChoice::Off),
        Some(r) => kodi.state().borrow().playback.as_ref().and_then(|p| p.subtitles.get(r).map(|s| SubtitleChoice::Track(SubtitleIndex(s.index.0)))),
    };
    if let Some(choice) = choice {
        let kodi = kodi.clone();
        spawn_logged("subtitle", async move { kodi.set_subtitle(choice, true).await });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use r3cui_sdk::testing::{entry, until};
    use crate::testing::FakeKodi;

    async fn driver(fake: &FakeKodi) -> Kodi {
        let cfg = fake.config();
        let kodi = Kodi::start(&entry(&SPEC, &[(field::HOST, &cfg.host), (field::WS_PORT, &cfg.ws_port.to_string())]));
        until(&mut kodi.status(), |s| s.health == Health::Up).await;
        kodi
    }

    #[tokio::test]
    async fn no_address_is_not_set_up() {
        let kodi = Kodi::start(&entry(&SPEC, &[]));
        assert_eq!(kodi.status().borrow().health, Health::NotSetUp);
        assert_eq!(kodi.ui().unwrap().get(), Stage::default());
    }

    #[tokio::test]
    async fn the_wall_shows_while_shown_and_follows_the_keys() {
        let fake = FakeKodi::start().await;
        let kodi = driver(&fake).await;
        let ui = kodi.ui().unwrap();
        let mut stage = ui.view.clone();
        assert_eq!(stage.borrow().library, None, "not shown yet");
        ui.send(UiIntent::Shown(true));
        let s = until(&mut stage, |s| s.library.is_some()).await;
        let lib = s.library.unwrap();
        assert_eq!((lib.list.heading.as_str(), lib.list.items.len(), lib.focus), (MOVIES_HEADING, 6, 0));
        assert!(lib.list.items[0].poster.contains("/image/"), "{}", lib.list.items[0].poster);
        kodi.remote().unwrap().send(RemoteKey::Down).unwrap();
        until(&mut stage, |s| s.library.as_ref().is_some_and(|l| l.focus == 1)).await;
        ui.send(UiIntent::Shown(false));
        until(&mut stage, |s| s.library.is_none()).await;
    }

    #[tokio::test]
    async fn playing_shows_now_playing_and_takes_the_ui_intents() {
        let fake = FakeKodi::start().await;
        let kodi = driver(&fake).await;
        let ui = kodi.ui().unwrap();
        ui.send(UiIntent::Shown(true));
        fake.play(39);
        let mut stage = ui.view.clone();
        let s = until(&mut stage, |s| s.media.as_ref().is_some_and(|m| m.subtitles.len() == 2)).await;
        assert_eq!(s.library, None, "the wall gives way to Now Playing");
        let m = s.media.unwrap();
        assert_eq!((m.title.as_str(), m.paused, m.subtitle_current), ("2001: A Space Odyssey", false, None));
        ui.send(UiIntent::Subtitle(Some(1)));
        until(&mut stage, |s| s.media.as_ref().is_some_and(|m| m.subtitle_current == Some(1))).await;
        ui.send(UiIntent::PlayPause);
        until(&mut stage, |s| s.media.as_ref().is_some_and(|m| m.paused)).await;
        kodi.remote().unwrap().send(RemoteKey::Stop).unwrap();
        until(&mut stage, |s| s.media.is_none()).await;
    }
}
