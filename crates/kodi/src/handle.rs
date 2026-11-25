//! `KodiHandle`: what the UI holds. It watches `KodiState`, and sends Kodi typed commands. Keys are fire-and-forget
//! (a failure lands in the log, never blocks); the rest can be awaited.

use crate::browse::{self, Browsing, Content};
use crate::client::Client;
use crate::methods::gui::GetInfoLabels;
use crate::methods::library::{FileItem, GetDirectory, MOVIE_FIELDS, SHOW_FIELDS};
use crate::wire::ArtRef;
use crate::config::KodiConfig;
use crate::engine::Query;
use crate::error::{Error, Result};
use crate::ids::{AudioStreamIndex, EpisodeId, MovieId, PlayerId, SubtitleIndex};
use crate::methods::input::{self, InputAction};
use crate::methods::player::{
    Open, OpenItem, OpenOptions, PlayPause, Seek, SeekTo, SetAudioStream, SetSubtitle, Stop, SubtitleTarget, SubtitleWord, Toggle,
};
use crate::rpc::Method;
use crate::state::KodiState;
use crate::wire::Time;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};

const COMMAND: Duration = Duration::from_secs(2);

/// Kodi's own navigation keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Up,
    Down,
    Left,
    Right,
    Select,
    Back,
    Home,
    ContextMenu,
    Info,
    Osd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleChoice {
    Off,
    Track(SubtitleIndex),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartAt {
    Beginning,
    Resume,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Media {
    Movie(MovieId),
    Episode(EpisodeId),
}

#[derive(Clone)]
pub struct KodiHandle {
    cfg: KodiConfig,
    state: watch::Receiver<KodiState>,
    client: Arc<Mutex<Option<Client>>>,
    /// Ask the service to read something again: for changes Kodi makes without telling (audio, subtitles).
    refresh: mpsc::UnboundedSender<Query>,
}

impl KodiHandle {
    pub(crate) fn new(cfg: KodiConfig, state: watch::Receiver<KodiState>, client: Arc<Mutex<Option<Client>>>, refresh: mpsc::UnboundedSender<Query>) -> Self {
        Self { cfg, state, client, refresh }
    }

    pub fn state(&self) -> watch::Receiver<KodiState> {
        self.state.clone()
    }

    pub fn config(&self) -> &KodiConfig {
        &self.cfg
    }

    /// Any method, typed; fails at once while Kodi isn't connected.
    pub async fn call<M: Method>(&self, method: M) -> Result<M::Output> {
        self.call_within(method, COMMAND).await
    }

    async fn call_within<M: Method>(&self, method: M, timeout: Duration) -> Result<M::Output> {
        let client = self.client.lock().unwrap().clone().ok_or(Error::Closed)?;
        client.call(&method, timeout).await
    }

    /// What Kodi's GUI is browsing now.
    pub async fn browsing(&self) -> Result<Browsing> {
        Ok(Browsing::from_labels(&self.call(GetInfoLabels::of(browse::LABELS)).await?))
    }

    /// The list `b` shows, in the GUI's order, with the fields the wall reads. A big library takes a while.
    pub async fn listing(&self, b: &Browsing) -> Result<Vec<FileItem>> {
        let properties = if b.content == Content::TvShows { SHOW_FIELDS } else { MOVIE_FIELDS };
        let list = GetDirectory { directory: b.path.clone(), media: "video", properties, sort: b.sort() };
        Ok(self.call_within(list, Duration::from_secs(15)).await?.files)
    }

    /// Where Kodi serves an artwork, asking the source for a `w` by `h` copy where it can make one.
    pub fn image_url(&self, art: &ArtRef, size: Option<(u32, u32)>) -> String {
        let art = size.map_or_else(|| art.clone(), |(w, h)| crate::art::sized(art, w, h));
        self.cfg.image_url(&art)
    }

    /// Send without waiting; a failure is logged.
    fn fire<M: Method>(&self, method: M) {
        let handle = self.clone();
        tokio::spawn(async move {
            if let Err(e) = handle.call(method).await {
                tracing::info!("kodi: {}: {e}", M::NAME);
            }
        });
    }

    fn player(&self) -> Result<PlayerId> {
        self.state.borrow().playback.as_ref().map(|p| p.player).ok_or(Error::NothingPlaying)
    }

    pub fn nav(&self, n: Nav) {
        match n {
            Nav::Up => self.fire(input::Up {}),
            Nav::Down => self.fire(input::Down {}),
            Nav::Left => self.fire(input::Left {}),
            Nav::Right => self.fire(input::Right {}),
            Nav::Select => self.fire(input::Select {}),
            Nav::Back => self.fire(input::Back {}),
            Nav::Home => self.fire(input::Home {}),
            Nav::ContextMenu => self.fire(input::ContextMenu {}),
            Nav::Info => self.fire(input::Info {}),
            Nav::Osd => self.fire(input::ShowOsd {}),
        }
    }

    pub fn action(&self, action: InputAction) {
        self.fire(input::ExecuteAction { action });
    }

    /// Pause or play what's playing; with nothing playing, Kodi's own play/pause (which plays a focused item).
    pub fn play_pause(&self) {
        match self.player() {
            Ok(playerid) => self.fire(PlayPause { playerid, play: Toggle::Toggle }),
            Err(_) => self.action(InputAction::PlayPause),
        }
    }

    pub fn stop(&self) {
        if let Ok(playerid) = self.player() {
            self.fire(Stop { playerid });
        }
    }

    pub async fn seek_to(&self, to: Duration) -> Result<()> {
        let playerid = self.player()?;
        self.call(Seek { playerid, value: SeekTo::Time { time: Time(to) } }).await?;
        Ok(())
    }

    pub async fn seek_by(&self, secs: i32) -> Result<()> {
        let playerid = self.player()?;
        self.call(Seek { playerid, value: SeekTo::Seconds { seconds: secs } }).await?;
        Ok(())
    }

    pub async fn set_audio(&self, stream: AudioStreamIndex) -> Result<()> {
        let playerid = self.player()?;
        self.call(SetAudioStream { playerid, stream }).await?;
        let _ = self.refresh.send(Query::Streams(playerid));
        Ok(())
    }

    /// Kodi keeps several seconds of the film read ahead, and drops the pictures of a subtitle track that was off when
    /// it read them, so a track switched on shows nothing until that runs out. `reload` seeks to where playback stands,
    /// which reads it again; for bitmap subtitles (Blu-ray PGS) that's the difference between seeing the next line
    /// and not.
    pub async fn set_subtitle(&self, choice: SubtitleChoice, reload: bool) -> Result<()> {
        let playerid = self.player()?;
        let (subtitle, enable) = match choice {
            SubtitleChoice::Off => (SubtitleTarget::Word(SubtitleWord::Off), false),
            SubtitleChoice::Track(i) => (SubtitleTarget::Index(i), true),
        };
        self.call(SetSubtitle { playerid, subtitle, enable }).await?;
        if reload && enable {
            let at = self.state.borrow().playback.as_ref().map(|p| p.timeline.position_at(Instant::now()));
            if let Some(at) = at {
                self.call(Seek { playerid, value: SeekTo::Time { time: Time(at) } }).await?;
            }
        }
        let _ = self.refresh.send(Query::Streams(playerid));
        Ok(())
    }

    pub async fn open(&self, media: Media, from: StartAt) -> Result<()> {
        let item = match media {
            Media::Movie(movieid) => OpenItem::Movie { movieid },
            Media::Episode(episodeid) => OpenItem::Episode { episodeid },
        };
        self.call(Open { item, options: OpenOptions { resume: from == StartAt::Resume } }).await?;
        Ok(())
    }
}
