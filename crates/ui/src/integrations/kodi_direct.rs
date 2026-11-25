//! The remote's side of r3cui's own Kodi client (`crates/kodi`), used when `kodi.mode` is "direct": where Kodi is,
//! what the screen shows of `KodiState`, and the remote's keys and pickers as Kodi commands. In "core" mode (the
//! default for now) none of this runs and Kodi comes through the core's Kodi integration.

use crate::app::settings;
use crate::input::KodiAction;
use crate::view::screen::Picker;
use coreapi::MediaState;
use kodi::ids::{AudioStreamIndex, SubtitleIndex};
use kodi::labels::{audio_format, audio_label, subtitle_label};
use kodi::methods::input::InputAction;
use kodi::{KodiConfig, KodiHandle, KodiState, Link, Nav, Playback, SubtitleChoice};
use std::time::Instant;

pub(crate) fn enabled() -> bool {
    std::env::var("UC_KODI_MODE").map_or_else(|_| settings::get().kodi_direct, |m| m == "direct")
}

/// Kodi's host from settings, or else from the core's Kodi entity (`kodi_driver.main.media_player.<host>`), so an
/// install that already has the core's Kodi integration needs no setup.
pub(crate) fn config(screen_entity: Option<&str>) -> Option<KodiConfig> {
    let s = settings::get();
    let host = std::env::var("UC_KODI_HOST").ok().or_else(|| s.kodi_host.clone()).or_else(|| {
        let id = screen_entity?;
        let host = id.rsplit_once("media_player.")?.1;
        (id.starts_with("kodi") && !host.is_empty()).then(|| host.to_string())
    })?;
    let mut cfg = KodiConfig::new(host);
    if let Some(p) = s.kodi_http_port {
        cfg.http_port = p;
    }
    if let Some(p) = s.kodi_ws_port {
        cfg.ws_port = p;
    }
    Some(cfg)
}

/// What Now Playing shows.
pub(crate) struct NowPlaying {
    pub(crate) title: String,
    pub(crate) paused: bool,
    pub(crate) duration: f64,
    pub(crate) position: f64,
    pub(crate) media_format: String,
    pub(crate) media_sub: String,
}

pub(crate) fn now_playing(state: &KodiState) -> NowPlaying {
    let Some(p) = &state.playback else {
        return NowPlaying { title: String::new(), paused: true, duration: 0.0, position: 0.0, media_format: String::new(), media_sub: String::new() };
    };
    let audio = p.audio_current.and_then(|i| p.audio.iter().find(|a| a.index == i));
    NowPlaying {
        title: p.item.title.clone(),
        paused: p.timeline.speed.is_paused(),
        duration: p.timeline.total.as_secs_f64(),
        position: p.timeline.position_at(Instant::now()).as_secs_f64(),
        media_format: audio.map(audio_format).unwrap_or_default(),
        media_sub: p.item.show.as_ref().map(|s| format!("Season {} · Episode {}", s.season, s.episode)).unwrap_or_default(),
    }
}

/// For the status page: whether the link is up, and what Kodi is doing.
pub(crate) fn health(state: &KodiState) -> (Option<bool>, Option<MediaState>) {
    match &state.link {
        Link::Up => {
            let doing = match &state.playback {
                Some(p) if p.timeline.speed.is_paused() => MediaState::Paused,
                Some(_) => MediaState::Playing,
                None => MediaState::On,
            };
            (Some(true), Some(doing))
        }
        Link::Unconfigured => (None, None),
        _ => (Some(false), Some(MediaState::Unavailable)),
    }
}

/// The poster and the backdrop to show, as full image URLs.
pub(crate) fn art(handle: &KodiHandle, state: &KodiState) -> (String, String) {
    let Some(p) = &state.playback else { return (String::new(), String::new()) };
    let a = &p.item.art;
    let url = |r: Option<&kodi::wire::ArtRef>| r.map(|r| handle.config().image_url(r)).unwrap_or_default();
    (url(a.poster.as_ref().or(a.show_poster.as_ref()).or(a.thumb.as_ref())), url(a.fanart.as_ref().or(a.show_fanart.as_ref())))
}

/// The picker's rows (label, detail) and the one on now.
pub(crate) fn tracks(state: &KodiState, picker: Picker) -> (Vec<(String, String)>, usize) {
    let Some(p) = &state.playback else { return (vec![], usize::MAX) };
    match picker {
        Picker::Audio => {
            let current = p.audio.iter().position(|a| Some(a.index) == p.audio_current).unwrap_or(usize::MAX);
            (p.audio.iter().map(|a| (audio_label(a), audio_format(a))).collect(), current)
        }
        Picker::Subtitles => {
            let mut rows = vec![("Off".to_string(), String::new())];
            rows.extend(p.subtitles.iter().map(|s| (subtitle_label(s), String::new())));
            let current = p.subtitle.and_then(|i| p.subtitles.iter().position(|s| s.index == i)).map_or(0, |i| i + 1);
            (rows, current)
        }
    }
}

/// A picker row chosen.
pub(crate) fn pick(handle: &KodiHandle, picker: Picker, row: usize) {
    let playback: Option<Playback> = handle.state().borrow().playback.clone();
    let Some(p) = playback else { return };
    let handle = handle.clone();
    tokio::spawn(async move {
        let done = match picker {
            Picker::Audio => match p.audio.get(row) {
                Some(a) => handle.set_audio(AudioStreamIndex(a.index.0)).await,
                None => return,
            },
            Picker::Subtitles => {
                let choice = match row {
                    0 => SubtitleChoice::Off,
                    n => match p.subtitles.get(n - 1) {
                        Some(s) => SubtitleChoice::Track(SubtitleIndex(s.index.0)),
                        None => return,
                    },
                };
                handle.set_subtitle(choice, true).await
            }
        };
        if let Err(e) = done {
            tracing::warn!("kodi {picker:?}: {e}");
        }
    });
}

/// A remote key for Kodi. Kodi's own keys where it has them; Menu is the context menu while browsing and the
/// on-screen display while playing.
pub(crate) fn press(handle: &KodiHandle, action: KodiAction) {
    let playing = handle.state().borrow().playback.is_some();
    match action {
        KodiAction::Up => handle.nav(Nav::Up),
        KodiAction::Down => handle.nav(Nav::Down),
        KodiAction::Left => handle.nav(Nav::Left),
        KodiAction::Right => handle.nav(Nav::Right),
        KodiAction::Select => handle.nav(Nav::Select),
        KodiAction::Back => handle.nav(Nav::Back),
        KodiAction::Home => handle.nav(Nav::Home),
        KodiAction::Menu => handle.nav(if playing { Nav::Osd } else { Nav::ContextMenu }),
        KodiAction::ChannelUp => handle.action(if playing { InputAction::ChapterOrBigStepForward } else { InputAction::PageUp }),
        KodiAction::ChannelDown => handle.action(if playing { InputAction::ChapterOrBigStepBack } else { InputAction::PageDown }),
        KodiAction::Previous => handle.action(InputAction::SkipPrevious),
        KodiAction::Next => handle.action(InputAction::SkipNext),
        KodiAction::PlayPause => handle.play_pause(),
        KodiAction::Stop => handle.stop(),
        KodiAction::Record => {}
    }
}

/// Seek by `by` seconds.
pub(crate) fn seek_by(handle: &KodiHandle, by: i32) {
    let handle = handle.clone();
    tokio::spawn(async move {
        if let Err(e) = handle.seek_by(by).await {
            tracing::warn!("kodi seek: {e}");
        }
    });
}
