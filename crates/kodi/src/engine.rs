//! The client's logic, with no IO: each event (connected, a notification, a query's answer) updates `KodiState` and
//! says which queries to make next. The service runs the queries and feeds their answers back in. Because it never
//! waits on anything, every rule here is tested with plain values.

use crate::error::Error;
use crate::ids::{EpisodeId, MediaId, MovieId, PlayerId, TvShowId};
use crate::methods::player::{ActivePlayer, PlayerKind, PlayerProps, PlayingItem};
use crate::notification::Notification;
use crate::state::{EpisodeOf, KodiState, Link, NowPlaying, Playback, Timeline};
use crate::wire::Speed;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Connected { server: Option<String> },
    Disconnected { error: Error, retry_in: Duration },
    Notified(Notification),
    Answer(Answer),
}

/// What to ask Kodi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Query {
    ActivePlayers,
    Item(PlayerId),
    /// Position, length and speed: cheap, asked after anything that moves the timeline.
    Timeline(PlayerId),
    /// Audio and subtitle streams, and whether it can seek.
    Streams(PlayerId),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    ActivePlayers(Vec<ActivePlayer>),
    Item(PlayerId, PlayingItem),
    /// `measured_at` is the moment Kodi's answer describes: halfway through the round trip.
    Timeline(PlayerId, PlayerProps, Instant),
    Streams(PlayerId, PlayerProps),
    Failed(Query, Error),
}

#[derive(Debug, Default)]
pub struct Engine {
    pub state: KodiState,
}

impl Engine {
    pub fn apply(&mut self, event: Event, now: Instant) -> Vec<Query> {
        match event {
            Event::Connected { server } => {
                self.state.link = Link::Up;
                self.state.server = server;
                // Notifications were missed while away: start again from what Kodi says now.
                vec![Query::ActivePlayers]
            }
            Event::Disconnected { error, retry_in } => {
                self.state.link = Link::Down { error, retry_in };
                self.state.playback = None;
                vec![]
            }
            Event::Notified(n) => self.notified(n, now),
            Event::Answer(a) => self.answered(a),
        }
    }

    fn notified(&mut self, n: Notification, now: Instant) -> Vec<Query> {
        match n {
            Notification::Play(e) | Notification::AvStart(e) => {
                let p = self.playback_for(e.player, PlayerKind::Video, now);
                p.timeline.speed = e.speed;
                vec![Query::Item(e.player), Query::Timeline(e.player), Query::Streams(e.player)]
            }
            Notification::AvChange(e) => vec![Query::Streams(e.player)],
            Notification::Pause(e) | Notification::Resume(e) | Notification::SpeedChanged(e) => {
                if let Some(p) = self.playing(e.player) {
                    // Hold the position where it stands at the new speed until Kodi says exactly.
                    p.timeline = Timeline { position: p.timeline.position_at(now), speed: e.speed, measured_at: now, ..p.timeline };
                }
                vec![Query::Timeline(e.player)]
            }
            Notification::Seek(s) => match (self.playing(s.player), s.time) {
                (Some(p), Some(time)) => {
                    p.timeline = Timeline { position: time.0, speed: s.speed, measured_at: now, ..p.timeline };
                    vec![]
                }
                _ => vec![Query::Timeline(s.player)],
            },
            Notification::Stop { .. } => {
                self.state.playback = None;
                vec![]
            }
            Notification::Screensaver(on) => {
                self.state.screensaver = on;
                vec![]
            }
            Notification::Wake => vec![Query::ActivePlayers],
            _ => vec![],
        }
    }

    fn answered(&mut self, a: Answer) -> Vec<Query> {
        match a {
            Answer::ActivePlayers(players) => {
                // The video player if there is one, else whatever plays.
                let Some(active) = players.iter().find(|p| p.kind == PlayerKind::Video).or(players.first()).cloned() else {
                    self.state.playback = None;
                    return vec![];
                };
                let id = active.playerid;
                if self.state.playback.as_ref().is_none_or(|p| p.player != id) {
                    self.state.playback = None;
                    self.playback_for(id, active.kind, Instant::now());
                }
                vec![Query::Item(id), Query::Timeline(id), Query::Streams(id)]
            }
            Answer::Item(player, item) => {
                if let Some(p) = self.playing(player) {
                    p.item = now_playing(&item);
                }
                vec![]
            }
            Answer::Timeline(player, props, measured_at) => {
                if let Some(p) = self.playing(player) {
                    let t = &mut p.timeline;
                    if let Some(time) = props.time {
                        (t.position, t.measured_at) = (time.0, measured_at);
                    }
                    if let Some(total) = props.totaltime {
                        t.total = total.0;
                    }
                    if let Some(speed) = props.speed {
                        t.speed = speed;
                    }
                }
                vec![]
            }
            Answer::Streams(player, props) => {
                if let Some(p) = self.playing(player) {
                    if let Some(a) = props.audiostreams {
                        p.audio = a;
                    }
                    p.audio_current = props.currentaudiostream.map(|a| a.index);
                    if let Some(s) = props.subtitles {
                        p.subtitles = s;
                    }
                    p.subtitle = props.currentsubtitle.filter(|_| props.subtitleenabled == Some(true)).map(|s| s.index);
                    p.can_seek = props.canseek.unwrap_or(p.can_seek);
                    p.live = props.live.unwrap_or(p.live);
                }
                vec![]
            }
            Answer::Failed(q, e) => {
                tracing::debug!("{q:?}: {e}");
                // Asking about a player that has gone: nothing plays any more.
                if matches!(e, Error::Rpc { .. }) && !matches!(q, Query::ActivePlayers) {
                    self.state.playback = None;
                }
                vec![]
            }
        }
    }

    fn playing(&mut self, player: PlayerId) -> Option<&mut Playback> {
        self.state.playback.as_mut().filter(|p| p.player == player)
    }

    /// The playback for `player`, made empty if there wasn't one for it.
    fn playback_for(&mut self, player: PlayerId, kind: PlayerKind, now: Instant) -> &mut Playback {
        if self.state.playback.as_ref().is_none_or(|p| p.player != player) {
            self.state.playback = Some(Playback {
                player,
                kind,
                item: NowPlaying::default(),
                timeline: Timeline { position: Duration::ZERO, total: Duration::ZERO, speed: Speed::NORMAL, measured_at: now },
                audio: vec![],
                audio_current: None,
                subtitles: vec![],
                subtitle: None,
                can_seek: false,
                live: false,
            });
        }
        self.state.playback.as_mut().unwrap()
    }
}

fn now_playing(item: &PlayingItem) -> NowPlaying {
    let id = item.id.and_then(|id| match item.kind.as_str() {
        "movie" => Some(MediaId::Movie(MovieId(id))),
        "episode" => Some(MediaId::Episode(EpisodeId(id))),
        "tvshow" => Some(MediaId::TvShow(TvShowId(id))),
        _ => None,
    });
    let show = (item.kind == "episode").then(|| EpisodeOf {
        show_title: item.show_title.clone(),
        season: item.season.max(0) as u32,
        episode: item.episode.max(0) as u32,
    });
    NowPlaying {
        id,
        title: if item.title.is_empty() { item.label.clone() } else { item.title.clone() },
        show,
        year: u16::try_from(item.year).ok().filter(|y| *y > 0),
        art: item.art.clone(),
        runtime: (item.runtime > 0).then(|| Duration::from_secs(item.runtime.into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::SubtitleIndex;
    use crate::notification::{NotifyItem, PlayerEvent, SeekEvent};
    use crate::wire::{SubtitleStream, Time};

    const P: PlayerId = PlayerId(1);

    fn event(speed: Speed) -> PlayerEvent {
        PlayerEvent { player: P, speed, item: NotifyItem::default() }
    }

    #[test]
    fn playing_starts_from_kodis_answers() {
        let (mut e, t) = (Engine::default(), Instant::now());
        assert_eq!(e.apply(Event::Connected { server: Some("Kodi 21.3".into()) }, t), vec![Query::ActivePlayers]);
        let active = vec![ActivePlayer { playerid: P, kind: PlayerKind::Video }];
        assert_eq!(e.apply(Event::Answer(Answer::ActivePlayers(active)), t), vec![Query::Item(P), Query::Timeline(P), Query::Streams(P)]);
        let item = PlayingItem { id: Some(39), kind: "movie".into(), title: "2001: A Space Odyssey".into(), year: 1968, ..Default::default() };
        e.apply(Event::Answer(Answer::Item(P, item)), t);
        let props = PlayerProps { time: Some(Time(Duration::from_secs(6837))), totaltime: Some(Time(Duration::from_secs(8931))), speed: Some(Speed::NORMAL), ..Default::default() };
        e.apply(Event::Answer(Answer::Timeline(P, props, t)), t);
        let p = e.state.playback.as_ref().unwrap();
        assert_eq!((p.item.title.as_str(), p.item.id, p.item.year), ("2001: A Space Odyssey", Some(MediaId::Movie(MovieId(39))), Some(1968)));
        assert_eq!(p.timeline.position_at(t + Duration::from_secs(2)), Duration::from_secs(6839));
    }

    #[test]
    fn pause_holds_the_position_and_seek_moves_it() {
        let (mut e, t) = (Engine::default(), Instant::now());
        e.apply(Event::Notified(Notification::Play(event(Speed::NORMAL))), t);
        e.apply(Event::Answer(Answer::Timeline(P, PlayerProps { time: Some(Time(Duration::from_secs(100))), ..Default::default() }, t)), t);
        let later = t + Duration::from_secs(5);
        assert_eq!(e.apply(Event::Notified(Notification::Pause(event(Speed::PAUSED))), later), vec![Query::Timeline(P)]);
        let tl = e.state.playback.as_ref().unwrap().timeline;
        assert_eq!(tl.position_at(later + Duration::from_secs(30)), Duration::from_secs(105), "a pause isn't time played");
        let seek = SeekEvent { player: P, speed: Speed::NORMAL, time: Some(Time(Duration::from_secs(3000))) };
        assert_eq!(e.apply(Event::Notified(Notification::Seek(seek)), later), vec![]);
        assert_eq!(e.state.playback.as_ref().unwrap().timeline.position_at(later), Duration::from_secs(3000));
        e.apply(Event::Notified(Notification::Stop { end: false }), later);
        assert!(e.state.playback.is_none());
    }

    #[test]
    fn subtitles_count_only_when_enabled() {
        let (mut e, t) = (Engine::default(), Instant::now());
        e.apply(Event::Notified(Notification::Play(event(Speed::NORMAL))), t);
        let sdh = SubtitleStream { index: SubtitleIndex(0), name: "SDH".into(), language: "eng".into(), forced: false, impaired: false, default: false };
        let props = |on| PlayerProps { subtitles: Some(vec![sdh.clone()]), currentsubtitle: Some(sdh.clone()), subtitleenabled: Some(on), ..Default::default() };
        e.apply(Event::Answer(Answer::Streams(P, props(false))), t);
        assert_eq!(e.state.playback.as_ref().unwrap().subtitle, None);
        e.apply(Event::Answer(Answer::Streams(P, props(true))), t);
        assert_eq!(e.state.playback.as_ref().unwrap().subtitle, Some(SubtitleIndex(0)));
    }

    #[test]
    fn losing_kodi_clears_playback() {
        let (mut e, t) = (Engine::default(), Instant::now());
        e.apply(Event::Notified(Notification::Play(event(Speed::NORMAL))), t);
        e.apply(Event::Disconnected { error: Error::Closed, retry_in: Duration::from_secs(1) }, t);
        assert!(e.state.playback.is_none() && matches!(e.state.link, Link::Down { .. }));
    }
}
