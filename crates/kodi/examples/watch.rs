//! Connect to a Kodi and print what the client knows as it changes:
//! `cargo run -p kodi --example watch -- kodi.lan [seconds]`.

use kodi::{KodiConfig, KodiService};
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let host = args.next().expect("usage: watch <host> [seconds]");
    let secs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(30);
    let handle = KodiService::start(KodiConfig::new(host));
    let mut state = handle.state();
    let start = tokio::time::Instant::now();
    let end = start + Duration::from_secs(secs);
    while tokio::time::timeout_at(end, state.changed()).await.is_ok_and(|r| r.is_ok()) {
        let s = state.borrow().clone();
        let now = Instant::now();
        let playing = s.playback.as_ref().map(|p| {
            let at = p.timeline.position_at(now).as_secs();
            let sub = p.subtitle.and_then(|i| p.subtitles.iter().find(|x| x.index == i)).map_or("off".into(), |x| format!("{} {}", x.language, x.name));
            format!(
                "{:?} {:?} at {}:{:02}:{:02} of {}s, speed {}, {} audio, subs {sub}",
                p.item.title, p.item.year, at / 3600, at / 60 % 60, at % 60, p.timeline.total.as_secs(), p.timeline.speed.0, p.audio.len()
            )
        });
        println!("{:>6.2}s {:?} {:?} {}", start.elapsed().as_secs_f32(), s.link, s.server, playing.unwrap_or_else(|| "nothing playing".into()));
    }
}
