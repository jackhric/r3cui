//! The real service and handle against `FakeKodi`.

use kodi::browse::Content;
use kodi::ids::{MovieId, SubtitleIndex};
use kodi::testing::FakeKodi;
use kodi::{KodiHandle, KodiService, KodiState, Link, Media, Nav, StartAt, SubtitleChoice};
use std::time::{Duration, Instant};
use tokio::time::sleep;
use wsclient::testing::{eventually, until};

async fn until_state(handle: &KodiHandle, what: &str, ok: impl Fn(&KodiState) -> bool) -> KodiState {
    until(&mut handle.state(), what, ok).await
}

async fn up() -> (FakeKodi, KodiHandle) {
    let fake = FakeKodi::start().await;
    let handle = KodiService::start(fake.config());
    until_state(&handle, "the link", |s| s.link == Link::Up).await;
    (fake, handle)
}

fn position(s: &KodiState) -> Duration {
    s.playback.as_ref().unwrap().timeline.position_at(Instant::now())
}

#[tokio::test]
async fn link_comes_up_with_nothing_playing() {
    let (fake, handle) = up().await;
    let s = until_state(&handle, "the server name", |s| s.server.is_some()).await;
    assert_eq!(s.server.as_deref(), Some("Kodi 21.3"));
    assert!(s.playback.is_none());
    eventually("the first queries", || async { fake.calls().contains(&"Player.GetActivePlayers".to_string()) }).await;
    assert!(fake.calls().contains(&"Application.GetProperties".to_string()));
    assert_eq!(fake.connections(), 1);
}

#[tokio::test]
async fn opening_a_movie_plays_it() {
    let (_fake, handle) = up().await;
    handle.open(Media::Movie(MovieId(39)), StartAt::Beginning).await.unwrap();
    let s = until_state(&handle, "the item and its length", |s| s.playback.as_ref().is_some_and(|p| !p.item.title.is_empty() && !p.timeline.total.is_zero())).await;
    let p = s.playback.unwrap();
    assert_eq!((p.item.title.as_str(), p.item.year), ("2001: A Space Odyssey", Some(1968)));
    assert_eq!(p.timeline.total, Duration::from_secs(8931));
    assert_eq!(p.item.runtime, Some(Duration::from_secs(8931)));
    assert!(!p.timeline.speed.is_paused());
    let first = position(&until_state(&handle, "streams", |s| s.playback.as_ref().is_some_and(|p| !p.audio.is_empty())).await);
    sleep(Duration::from_millis(400)).await;
    assert!(position(&handle.state().borrow()) >= first + Duration::from_millis(300), "the timeline runs");
}

#[tokio::test]
async fn resume_starts_at_the_resume_point() {
    let (_fake, handle) = up().await;
    handle.open(Media::Movie(MovieId(39)), StartAt::Resume).await.unwrap();
    let s = until_state(&handle, "a timeline", |s| s.playback.as_ref().is_some_and(|p| !p.timeline.total.is_zero())).await;
    assert!(position(&s) >= Duration::from_secs(6837));
}

#[tokio::test]
async fn pause_and_seek() {
    let (_fake, handle) = up().await;
    handle.open(Media::Movie(MovieId(39)), StartAt::Beginning).await.unwrap();
    until_state(&handle, "playing", |s| s.playback.as_ref().is_some_and(|p| !p.timeline.total.is_zero())).await;
    handle.play_pause();
    until_state(&handle, "paused", |s| s.playback.as_ref().is_some_and(|p| p.timeline.speed.is_paused())).await;
    handle.seek_to(Duration::from_secs(5000)).await.unwrap();
    let s = until_state(&handle, "the seek", |s| position(s) >= Duration::from_secs(5000)).await;
    assert!(position(&s) < Duration::from_secs(5002));
    handle.seek_by(-60).await.unwrap();
    until_state(&handle, "the relative seek", |s| position(s) < Duration::from_secs(4945) && position(s) >= Duration::from_secs(4938)).await;
    handle.play_pause();
    until_state(&handle, "resumed", |s| s.playback.as_ref().is_some_and(|p| !p.timeline.speed.is_paused())).await;
}

#[tokio::test]
async fn subtitle_changes_are_read_back_without_a_notification() {
    let (fake, handle) = up().await;
    handle.open(Media::Movie(MovieId(39)), StartAt::Beginning).await.unwrap();
    let s = until_state(&handle, "streams", |s| s.playback.as_ref().is_some_and(|p| p.subtitles.len() == 2)).await;
    let p = s.playback.unwrap();
    assert_eq!(p.subtitles[1].name, "English SDH");
    assert_eq!((p.audio.len(), p.subtitle), (1, None));

    handle.set_subtitle(SubtitleChoice::Track(SubtitleIndex(1)), false).await.unwrap();
    until_state(&handle, "the SDH track", |s| s.playback.as_ref().is_some_and(|p| p.subtitle == Some(SubtitleIndex(1)))).await;
    handle.set_subtitle(SubtitleChoice::Off, false).await.unwrap();
    until_state(&handle, "subtitles off", |s| s.playback.as_ref().is_some_and(|p| p.subtitle.is_none())).await;

    handle.set_subtitle(SubtitleChoice::Track(SubtitleIndex(0)), true).await.unwrap();
    until_state(&handle, "the English track", |s| s.playback.as_ref().is_some_and(|p| p.subtitle == Some(SubtitleIndex(0)))).await;
    assert!(fake.calls().contains(&"Player.Seek".to_string()), "the reload seeks");
}

#[tokio::test]
async fn browsing_and_listing_mirror_the_gui() {
    let (_fake, handle) = up().await;
    let b = handle.browsing().await.unwrap();
    assert_eq!(b.content, Content::Movies);
    assert_eq!((b.path.as_str(), b.sort_label.as_str(), b.ascending, b.focus), ("videodb://movies/titles/", "Title", true, Some(39)));
    let titles: Vec<_> = handle.listing(&b).await.unwrap().into_iter().map(|f| f.title).collect();
    assert_eq!(titles, ["2001: A Space Odyssey", "Alien", "Arrival", "Blade Runner", "A Bronx Tale", "The Godfather"]);

    let mut descending = b.clone();
    descending.ascending = false;
    let files = handle.listing(&descending).await.unwrap();
    assert_eq!(files[0].title, "The Godfather");
    assert_eq!((files[5].id, files[5].year), (Some(39), 1968));
    assert!(files[0].art.poster.as_ref().unwrap().0.ends_with("poster.jpg/"));

    handle.nav(Nav::Down);
    eventually("the highlight to move", || async { handle.browsing().await.unwrap().focus == Some(21) }).await;
    handle.nav(Nav::Up);
    eventually("the highlight to move back", || async { handle.browsing().await.unwrap().focus == Some(39) }).await;
    handle.nav(Nav::Down);
    eventually("the highlight to move", || async { handle.browsing().await.unwrap().focus == Some(21) }).await;
    handle.nav(Nav::Select);
    let s = until_state(&handle, "the focused movie", |s| s.playback.as_ref().is_some_and(|p| p.item.title == "Alien")).await;
    assert_eq!(s.playback.unwrap().item.year, Some(1979));
}

#[tokio::test]
async fn kodi_side_play_and_stop_arrive_as_notifications() {
    let (fake, handle) = up().await;
    fake.play(12);
    until_state(&handle, "Blade Runner", |s| s.playback.as_ref().is_some_and(|p| p.item.title == "Blade Runner")).await;
    fake.stop();
    until_state(&handle, "nothing playing", |s| s.playback.is_none()).await;
    assert!(handle.seek_to(Duration::from_secs(1)).await.is_err());
}

#[tokio::test]
async fn unknown_methods_are_rejected() {
    use kodi::methods::gui::GetGuiProperties;
    let (_fake, handle) = up().await;
    let err = handle.call(GetGuiProperties { properties: &[] }).await.unwrap_err();
    assert!(matches!(err, kodi::Error::Rpc { code: -32601, .. }), "{err:?}");
}

#[tokio::test]
async fn a_kicked_client_reconnects_and_recovers() {
    let (fake, handle) = up().await;
    handle.open(Media::Movie(MovieId(39)), StartAt::Beginning).await.unwrap();
    until_state(&handle, "playing", |s| s.playback.is_some()).await;
    fake.kick();
    eventually("a second connection", || async { fake.accepted() >= 2 }).await;
    let s = until_state(&handle, "the state recovered", |s| s.link == Link::Up && s.playback.as_ref().is_some_and(|p| p.item.title == "2001: A Space Odyssey" && !p.timeline.total.is_zero())).await;
    assert!(position(&s) < Duration::from_secs(60));
    assert_eq!(fake.connections(), 1);
}
