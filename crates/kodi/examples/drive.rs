//! Drive a Kodi through `KodiHandle` and print the state after each command:
//! `cargo run -p kodi --example drive -- kodi.lan <movie id>`. Plays the film from its resume point, so put back the
//! resume point, play count and last-played date afterwards if they matter.

use kodi::ids::{AudioStreamIndex, MovieId, SubtitleIndex};
use kodi::{KodiConfig, KodiService, Media, StartAt, SubtitleChoice};
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let host = args.next().expect("usage: drive <host> <movie id>");
    let movie = MovieId(args.next().and_then(|s| s.parse().ok()).expect("a movie id"));
    let kodi = KodiService::start(KodiConfig::new(host));
    let mut state = kodi.state();
    state.wait_for(|s| s.link == kodi::Link::Up).await.unwrap();
    let show = |what: &str| {
        let s = state.borrow().clone();
        match s.playback {
            Some(p) => {
                let at = p.timeline.position_at(Instant::now()).as_secs();
                let audio = p.audio_current.and_then(|i| p.audio.iter().find(|a| a.index == i)).map_or("?".into(), |a| a.name.clone());
                let sub = p.subtitle.map_or("off".into(), |i| format!("{}", i.0));
                println!("{what:<26} {} at {}:{:02}:{:02}, speed {}, audio {audio:.40}, subtitle {sub}", p.item.title, at / 3600, at / 60 % 60, at % 60, p.timeline.speed.0);
            }
            None => println!("{what:<26} nothing playing"),
        }
    };
    let pause = || tokio::time::sleep(Duration::from_secs(3));
    kodi.open(Media::Movie(movie), StartAt::Resume).await.unwrap();
    pause().await;
    show("opened from resume");
    kodi.set_subtitle(SubtitleChoice::Track(SubtitleIndex(0)), true).await.unwrap();
    pause().await;
    show("subtitle 0, reloaded");
    kodi.seek_by(-10).await.unwrap();
    pause().await;
    show("back 10 s");
    kodi.set_audio(AudioStreamIndex(1)).await.unwrap();
    pause().await;
    show("audio 1");
    kodi.set_audio(AudioStreamIndex(0)).await.unwrap();
    kodi.set_subtitle(SubtitleChoice::Off, false).await.unwrap();
    kodi.play_pause();
    pause().await;
    show("audio 0, subs off, paused");
    kodi.stop();
    pause().await;
    show("stopped");
}
