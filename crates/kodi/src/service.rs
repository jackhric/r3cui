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
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use wsclient::{Connection, Session};

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
        let (refresh_tx, refresh) = mpsc::unbounded_channel();
        tokio::spawn(run(cfg.clone(), state_tx, slot.clone(), refresh));
        KodiHandle::new(cfg, state_rx, slot, refresh_tx)
    }
}

async fn run(cfg: KodiConfig, state: watch::Sender<KodiState>, slot: Arc<Mutex<Option<Client>>>, refresh: mpsc::UnboundedReceiver<Query>) {
    wsclient::reconnect(&mut Following { cfg, engine: Engine::default(), state, slot, refresh }).await;
}

struct Following {
    cfg: KodiConfig,
    engine: Engine,
    state: watch::Sender<KodiState>,
    slot: Arc<Mutex<Option<Client>>>,
    refresh: mpsc::UnboundedReceiver<Query>,
}

impl Following {
    fn publish(&self) {
        self.state.send_replace(self.engine.state.clone());
    }

    /// One connection's life; returns why it ended.
    async fn follow(&mut self, conn: Connection) -> Error {
        let Connection { out, mut incoming } = conn;
        let client = Client::new(out);
        *self.slot.lock().unwrap() = Some(client.clone());
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
                            ask(self.engine.apply(Event::Notified(n), Instant::now()));
                            self.publish();
                        }
                        Err(e) => tracing::debug!("kodi frame: {e}"),
                    }
                }
                Some(server) = server_rx.recv() => {
                    tracing::info!("kodi: connected to {}", server.as_deref().unwrap_or("Kodi"));
                    ask(self.engine.apply(Event::Connected { server }, Instant::now()));
                    self.publish();
                }
                Some(q) = self.refresh.recv() => ask(vec![q]),
                Some(answer) = answers.recv() => {
                    ask(self.engine.apply(Event::Answer(answer), Instant::now()));
                    self.publish();
                }
            }
        }
    }
}

impl Session for Following {
    type Error = Error;

    fn connecting(&mut self, attempt: u32) -> bool {
        self.engine.state.link = Link::Connecting { attempt };
        self.publish();
        true
    }

    async fn connect(&mut self) -> Result<Connection, Error> {
        Ok(wsclient::connect(self.cfg.ws_url()).await?)
    }

    async fn serve(&mut self, conn: Connection) -> Error {
        let error = self.follow(conn).await;
        *self.slot.lock().unwrap() = None;
        error
    }

    fn lost(&mut self, error: Error, retry_in: Duration) {
        tracing::info!("kodi: {error}; again in {retry_in:?}");
        self.engine.apply(Event::Disconnected { error, retry_in }, Instant::now());
        self.publish();
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
