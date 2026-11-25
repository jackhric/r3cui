//! The running client: connects (and reconnects, backing off), sorts frames into replies and notifications, runs
//! the engine and the queries it asks for, and publishes `KodiState`.

use crate::client::Client;
use crate::config::KodiConfig;
use crate::engine::{Answer, Engine, Event, Query};
use crate::error::Error;
use crate::handle::KodiHandle;
use crate::methods::player::{GetActivePlayers, GetItem, GetProperties, ItemField, PlayerProperty};
use crate::methods::system::{AppProperty, GetApplicationProperties};
use crate::notification::Notification;
use crate::rpc::Incoming;
use crate::state::{KodiState, Link};
use crate::transport;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};

const READ: Duration = Duration::from_secs(4);
const ITEM_FIELDS: &[ItemField] = &[ItemField::Title, ItemField::ShowTitle, ItemField::Season, ItemField::Episode, ItemField::Year, ItemField::Runtime, ItemField::Art, ItemField::File];
const TIMELINE: &[PlayerProperty] = &[PlayerProperty::Time, PlayerProperty::TotalTime, PlayerProperty::Speed];
const STREAMS: &[PlayerProperty] = {
    use PlayerProperty::*;
    &[AudioStreams, CurrentAudioStream, Subtitles, CurrentSubtitle, SubtitleEnabled, CanSeek, Live]
};

pub struct KodiService;

impl KodiService {
    /// Start following Kodi on the current tokio runtime.
    pub fn start(cfg: KodiConfig) -> KodiHandle {
        let (state_tx, state_rx) = watch::channel(KodiState::default());
        let slot: Arc<Mutex<Option<Client>>> = Arc::default();
        tokio::spawn(run(cfg.clone(), state_tx, slot.clone()));
        KodiHandle::new(cfg, state_rx, slot)
    }
}

async fn run(cfg: KodiConfig, state: watch::Sender<KodiState>, slot: Arc<Mutex<Option<Client>>>) {
    let mut engine = Engine::default();
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        engine.state.link = Link::Connecting { attempt };
        state.send_replace(engine.state.clone());
        let error = match transport::connect(&cfg.ws_url()).await {
            Ok(conn) => {
                let up = Instant::now();
                let error = connected(conn, &mut engine, &state, &slot).await;
                *slot.lock().unwrap() = None;
                // A connection that stayed up a while starts the backoff over.
                if up.elapsed() > Duration::from_secs(30) {
                    attempt = 1;
                }
                error
            }
            Err(e) => e,
        };
        let retry_in = backoff(attempt);
        tracing::info!("kodi: {error}; again in {retry_in:?}");
        engine.apply(Event::Disconnected { error, retry_in }, Instant::now());
        state.send_replace(engine.state.clone());
        tokio::time::sleep(retry_in).await;
    }
}

fn backoff(attempt: u32) -> Duration {
    const STEPS: [u64; 5] = [500, 1000, 2000, 4000, 8000];
    Duration::from_millis(STEPS.get(attempt.saturating_sub(1) as usize).copied().unwrap_or(15_000))
}

/// One connection's life; returns why it ended.
async fn connected(conn: transport::Connection, engine: &mut Engine, state: &watch::Sender<KodiState>, slot: &Arc<Mutex<Option<Client>>>) -> Error {
    let transport::Connection { out, mut incoming } = conn;
    let client = Client::new(out);
    *slot.lock().unwrap() = Some(client.clone());
    let (answers_tx, mut answers) = mpsc::unbounded_channel::<Answer>();
    let ask = |queries: Vec<Query>| {
        for q in queries {
            tokio::spawn(query(client.clone(), q, answers_tx.clone()));
        }
    };

    // Who Kodi is, asked alongside the first frames: the reader must already be running for the answer to arrive.
    let (server_tx, mut server_rx) = mpsc::unbounded_channel();
    {
        let client = client.clone();
        tokio::spawn(async move {
            let name = client
                .call(&GetApplicationProperties { properties: &[AppProperty::Name, AppProperty::Version] }, READ)
                .await
                .map(|a| format!("{} {}.{}", a.name, a.version.major, a.version.minor))
                .ok();
            let _ = server_tx.send(name);
        });
    }
    loop {
        tokio::select! {
            frame = incoming.recv() => {
                let Some(frame) = frame else { return Error::Closed };
                match Incoming::parse(&frame) {
                    Ok(Incoming::Reply { id, result }) => client.deliver(id, result),
                    Ok(Incoming::Notification { method, params }) => {
                        let n = Notification::parse(&method, &params);
                        tracing::debug!("kodi: {n:?}");
                        if n == Notification::Leaving {
                            return Error::Closed;
                        }
                        ask(engine.apply(Event::Notified(n), Instant::now()));
                        state.send_replace(engine.state.clone());
                    }
                    Err(e) => tracing::debug!("kodi frame: {e}"),
                }
            }
            Some(server) = server_rx.recv() => {
                tracing::info!("kodi: connected to {}", server.as_deref().unwrap_or("Kodi"));
                ask(engine.apply(Event::Connected { server }, Instant::now()));
                state.send_replace(engine.state.clone());
            }
            Some(answer) = answers.recv() => {
                ask(engine.apply(Event::Answer(answer), Instant::now()));
                state.send_replace(engine.state.clone());
            }
        }
    }
}

async fn query(client: Client, q: Query, answers: mpsc::UnboundedSender<Answer>) {
    let answer = match q {
        Query::ActivePlayers => client.call(&GetActivePlayers {}, READ).await.map(Answer::ActivePlayers),
        Query::Item(p) => client.call(&GetItem { playerid: p, properties: ITEM_FIELDS }, READ).await.map(|r| Answer::Item(p, r.item)),
        Query::Timeline(p) => {
            let sent = Instant::now();
            let r = client.call(&GetProperties { playerid: p, properties: TIMELINE }, READ).await;
            // Kodi read its clock about halfway through the round trip.
            r.map(|props| Answer::Timeline(p, props, sent + sent.elapsed() / 2))
        }
        Query::Streams(p) => client.call(&GetProperties { playerid: p, properties: STREAMS }, READ).await.map(|props| Answer::Streams(p, props)),
    };
    let _ = answers.send(answer.unwrap_or_else(|e| Answer::Failed(q, e)));
}
