//! `KodiHandle`: what the UI holds. It watches `KodiState`, and sends Kodi typed calls.

use crate::client::Client;
use crate::config::KodiConfig;
use crate::error::{Error, Result};
use crate::rpc::Method;
use crate::state::KodiState;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;

const COMMAND: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub struct KodiHandle {
    cfg: KodiConfig,
    state: watch::Receiver<KodiState>,
    client: Arc<Mutex<Option<Client>>>,
}

impl KodiHandle {
    pub(crate) fn new(cfg: KodiConfig, state: watch::Receiver<KodiState>, client: Arc<Mutex<Option<Client>>>) -> Self {
        Self { cfg, state, client }
    }

    pub fn state(&self) -> watch::Receiver<KodiState> {
        self.state.clone()
    }

    pub fn config(&self) -> &KodiConfig {
        &self.cfg
    }

    /// Any method, typed; fails at once while Kodi isn't connected.
    pub async fn call<M: Method>(&self, method: M) -> Result<M::Output> {
        let client = self.client.lock().unwrap().clone().ok_or(Error::Closed)?;
        client.call(&method, COMMAND).await
    }
}
