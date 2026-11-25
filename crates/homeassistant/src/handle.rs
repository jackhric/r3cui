//! `HaHandle`: what the UI holds. It watches `HaState`, changes what is watched, and calls Home Assistant's services.

use crate::client::Client;
use crate::config::HaConfig;
use crate::error::{Error, Result};
use crate::ids::EntityId;
use crate::state::{EntityState, HaState};
use crate::wire;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;

const CALL: Duration = Duration::from_secs(5);
const STATES: Duration = Duration::from_secs(15);

#[derive(Clone)]
pub struct HaHandle {
    cfg: HaConfig,
    state: watch::Receiver<HaState>,
    client: Arc<Mutex<Option<Client>>>,
    watched: Arc<watch::Sender<Vec<EntityId>>>,
}

impl HaHandle {
    pub(crate) fn new(cfg: HaConfig, state: watch::Receiver<HaState>, client: Arc<Mutex<Option<Client>>>, watched: watch::Sender<Vec<EntityId>>) -> Self {
        Self { cfg, state, client, watched: Arc::new(watched) }
    }

    pub fn state(&self) -> watch::Receiver<HaState> {
        self.state.clone()
    }

    pub fn config(&self) -> &HaConfig {
        &self.cfg
    }

    /// Follow these entities instead; the service subscribes again, on this connection or the next.
    pub fn watch(&self, ids: Vec<EntityId>) {
        self.watched.send_if_modified(|current| {
            let changed = *current != ids;
            if changed {
                *current = ids;
            }
            changed
        });
    }

    async fn call(&self, kind: &str, fields: Value, timeout: Duration) -> Result<Value> {
        let client = self.client.lock().unwrap().clone().ok_or(Error::Closed)?;
        client.call(kind, fields, timeout).await
    }

    pub async fn call_service(&self, domain: &str, service: &str, entity: &EntityId, data: Value) -> Result<()> {
        let data = if data.is_null() { json!({}) } else { data };
        let fields = json!({ "domain": domain, "service": service, "service_data": data, "target": { "entity_id": entity } });
        self.call("call_service", fields, CALL).await?;
        Ok(())
    }

    /// Every entity in Home Assistant, for pickers.
    pub async fn states(&self) -> Result<Vec<(EntityId, EntityState)>> {
        wire::states(self.call("get_states", json!({}), STATES).await?)
    }

    pub async fn volume_set(&self, entity: &EntityId, level: f64) -> Result<()> {
        let level = level.clamp(0.0, 1.0);
        self.call_service("media_player", "volume_set", entity, json!({ "volume_level": level })).await
    }

    pub async fn volume_up(&self, entity: &EntityId) -> Result<()> {
        self.call_service("media_player", "volume_up", entity, Value::Null).await
    }

    pub async fn volume_down(&self, entity: &EntityId) -> Result<()> {
        self.call_service("media_player", "volume_down", entity, Value::Null).await
    }

    pub async fn volume_mute(&self, entity: &EntityId, muted: bool) -> Result<()> {
        self.call_service("media_player", "volume_mute", entity, json!({ "is_volume_muted": muted })).await
    }

    pub async fn select_source(&self, entity: &EntityId, source: &str) -> Result<()> {
        self.call_service("media_player", "select_source", entity, json!({ "source": source })).await
    }

    pub async fn turn_on(&self, entity: &EntityId) -> Result<()> {
        self.call_service("media_player", "turn_on", entity, Value::Null).await
    }

    pub async fn turn_off(&self, entity: &EntityId) -> Result<()> {
        self.call_service("media_player", "turn_off", entity, Value::Null).await
    }
}
