//! The running remote channel: connects (and reconnects, backing off), answers the TV's handshake and pings, and
//! publishes `TvState`.

use crate::channel::Channel;
use crate::credentials::Credentials;
use crate::endpoint::Endpoint;
use crate::error::{Error, Result};
use crate::protocol::remote::{feature, DeviceInfo, RemoteMessage};
use crate::protocol::{Direction, KeyCode};
use crate::state::{Link, TvState, Volume};
use crate::tls::{client_config, dial};
use rustls::ClientConfig;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use wsclient::Backoff;

/// The TV pings every 5 s when nothing else is said; three missed pings and it hangs up, so we do the same.
const IDLE: Duration = Duration::from_secs(16);

type Outbox = Arc<Mutex<Option<mpsc::UnboundedSender<RemoteMessage>>>>;

/// Follows a TV and sends it commands. Cheap to clone; the connection ends when the last clone is dropped.
#[derive(Clone)]
pub struct AndroidTvHandle {
    state: watch::Receiver<TvState>,
    outbox: Outbox,
}

/// Starts following the TV on the current tokio runtime. It connects in the background and keeps doing so.
pub fn connect(endpoint: impl Into<Endpoint>, credentials: &Credentials) -> Result<AndroidTvHandle> {
    let config = client_config(credentials)?;
    let (state_tx, state) = watch::channel(TvState::default());
    let outbox = Outbox::default();
    tokio::spawn(run(endpoint.into(), config, state_tx, outbox.clone()));
    Ok(AndroidTvHandle { state, outbox })
}

impl AndroidTvHandle {
    pub fn state(&self) -> TvState {
        self.state.borrow().clone()
    }

    pub fn watch(&self) -> watch::Receiver<TvState> {
        self.state.clone()
    }

    pub fn send_key(&self, key: KeyCode, direction: Direction) -> Result<()> {
        self.send(RemoteMessage::key(key, direction))
    }

    /// A short press: down and up.
    pub fn press(&self, key: KeyCode) -> Result<()> {
        self.send_key(key, Direction::Short)
    }

    /// Opens an app link such as `https://www.netflix.com/title.` or an app's `market://` URI.
    pub fn launch(&self, app_link: &str) -> Result<()> {
        self.send(RemoteMessage::launch(app_link))
    }

    fn send(&self, message: RemoteMessage) -> Result<()> {
        let outbox = self.outbox.lock().unwrap();
        outbox.as_ref().ok_or(Error::NotConnected)?.send(message).map_err(|_| Error::NotConnected)
    }
}

async fn run(endpoint: Endpoint, config: Arc<ClientConfig>, state: watch::Sender<TvState>, outbox: Outbox) {
    let mut backoff = Backoff::new();
    loop {
        backoff.begin_attempt();
        let started = Instant::now();
        let ended = tokio::select! {
            ended = follow(&endpoint, config.clone(), &state, &outbox) => ended,
            () = state.closed() => return,
        };
        *outbox.lock().unwrap() = None;
        backoff.connection_ended(started.elapsed());
        let link = if matches!(ended, Error::NotPaired) { Link::NotPaired } else { Link::Down };
        tracing::debug!(host = %endpoint.host, "androidtv connection ended: {ended}");
        state.send_replace(TvState::down(link));
        tokio::select! {
            () = tokio::time::sleep(backoff.delay()) => {}
            () = state.closed() => return,
        }
    }
}

/// One connection's life; returns why it ended.
async fn follow(endpoint: &Endpoint, config: Arc<ClientConfig>, state: &watch::Sender<TvState>, outbox: &Outbox) -> Error {
    let tv = match dial(endpoint, endpoint.remote_port, config).await {
        Ok(tv) => tv,
        Err(e) => return e,
    };
    let mut channel = Channel::new(tv.stream);
    let (commands, mut queued) = mpsc::unbounded_channel();
    let mut active = feature::WANTED;
    let result: Result<()> = async {
        loop {
            tokio::select! {
                incoming = tokio::time::timeout(IDLE, channel.recv::<RemoteMessage>()) => {
                    let message = incoming.map_err(|_| Error::Timeout)??;
                    if let Some(reply) = react(&message, &mut active, state) {
                        channel.send(&reply).await?;
                    }
                    if message.start.is_some() {
                        *outbox.lock().unwrap() = Some(commands.clone());
                    }
                }
                Some(command) = queued.recv() => channel.send(&command).await?,
            }
        }
    }
    .await;
    result.err().unwrap_or(Error::Closed)
}

/// What to say back to `message`, after taking what it tells us into the state.
fn react(message: &RemoteMessage, active: &mut i32, state: &watch::Sender<TvState>) -> Option<RemoteMessage> {
    if let Some(configure) = &message.configure {
        *active = feature::WANTED & configure.code1;
        let model = configure.device_info.as_ref().map(|d| format!("{} {}", d.vendor, d.model).trim().to_string());
        state.send_modify(|s| s.model = model);
        return Some(RemoteMessage::configure(*active, DeviceInfo::this_client()));
    }
    if message.set_active.is_some() {
        return Some(RemoteMessage::set_active(*active));
    }
    if let Some(ping) = &message.ping_request {
        return Some(RemoteMessage::pong(ping));
    }
    if let Some(start) = &message.start {
        state.send_modify(|s| {
            s.on = start.started;
            s.link = Link::Up;
        });
    }
    if let Some(app) = message.ime_key_inject.as_ref().and_then(|i| i.app_info.as_ref()).filter(|a| !a.app_package.is_empty()) {
        state.send_if_modified(|s| set(&mut s.current_app, Some(app.app_package.clone())));
    }
    if let Some(v) = &message.set_volume_level {
        state.send_if_modified(|s| set(&mut s.volume, Some(Volume { level: v.volume_level, max: v.volume_max, muted: v.volume_muted })));
    }
    if let Some(error) = &message.error {
        tracing::warn!(value = error.value, "androidtv reported an error");
    }
    None
}

fn set<T: PartialEq>(slot: &mut T, value: T) -> bool {
    let changed = *slot != value;
    *slot = value;
    changed
}
