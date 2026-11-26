//! `HaHandle`: what the UI holds. It watches `HaState`, changes what is watched, and calls Home Assistant's services.

use crate::client::Client;
use crate::config::HaConfig;
use crate::error::{Error, Result};
use crate::ids::EntityId;
use crate::protocol::{attribute, domain, field, service, MessageType};
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

    async fn call(&self, kind: MessageType, fields: Value, timeout: Duration) -> Result<Value> {
        let client = self.client.lock().unwrap().clone().ok_or(Error::Closed)?;
        client.call(kind, fields, timeout).await
    }

    pub async fn call_service(&self, domain: &str, service: &str, entity: &EntityId, data: Value) -> Result<()> {
        let data = if data.is_null() { json!({}) } else { data };
        let fields = json!({ field::DOMAIN: domain, field::SERVICE: service, field::SERVICE_DATA: data, field::TARGET: { field::ENTITY_ID: entity } });
        self.call(MessageType::CallService, fields, CALL).await?;
        Ok(())
    }

    /// Every entity in Home Assistant, for pickers.
    pub async fn states(&self) -> Result<Vec<(EntityId, EntityState)>> {
        wire::states(self.call(MessageType::GetStates, json!({}), STATES).await?)
    }

    pub async fn volume_set(&self, entity: &EntityId, level: f64) -> Result<()> {
        let level = level.clamp(0.0, 1.0);
        self.call_service(domain::MEDIA_PLAYER, service::VOLUME_SET, entity, json!({ attribute::VOLUME_LEVEL: level })).await
    }

    pub async fn volume_up(&self, entity: &EntityId) -> Result<()> {
        self.call_service(domain::MEDIA_PLAYER, service::VOLUME_UP, entity, Value::Null).await
    }

    pub async fn volume_down(&self, entity: &EntityId) -> Result<()> {
        self.call_service(domain::MEDIA_PLAYER, service::VOLUME_DOWN, entity, Value::Null).await
    }

    pub async fn volume_mute(&self, entity: &EntityId, muted: bool) -> Result<()> {
        self.call_service(domain::MEDIA_PLAYER, service::VOLUME_MUTE, entity, json!({ attribute::IS_VOLUME_MUTED: muted })).await
    }

    pub async fn select_source(&self, entity: &EntityId, source: &str) -> Result<()> {
        self.call_service(domain::MEDIA_PLAYER, service::SELECT_SOURCE, entity, json!({ attribute::SOURCE: source })).await
    }

    pub async fn turn_on(&self, entity: &EntityId) -> Result<()> {
        self.call_service(domain::MEDIA_PLAYER, service::TURN_ON, entity, Value::Null).await
    }

    pub async fn turn_off(&self, entity: &EntityId) -> Result<()> {
        self.call_service(domain::MEDIA_PLAYER, service::TURN_OFF, entity, Value::Null).await
    }
}
