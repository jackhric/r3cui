//! The remote's software: its version, firmware updates and web-configurator access, typed after `core-api`'s AsyncAPI
//! spec. A request is a struct that says which message it is and what comes back (`Request`); `Core::call` sends it.

use crate::{messages, Core, Response};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;

pub trait Request: Serialize {
    const MSG: &'static str;
    type Reply: DeserializeOwned;
}

/// Why a call produced no reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallError {
    /// The core answered with an error code.
    Refused { code: i64, message: String },
    /// No answer: the connection is gone, or the answer was not what the spec says.
    Lost(String),
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused { code, message } => write!(f, "core refused with {code} {message}"),
            Self::Lost(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for CallError {}

impl Core {
    pub async fn call<R: Request>(&self, request: &R) -> Result<R::Reply, CallError> {
        let data = serde_json::to_value(request).map_err(|e| CallError::Lost(e.to_string()))?;
        let Response { code, data, .. } = self.request(R::MSG, data).await.map_err(|e| CallError::Lost(e.to_string()))?;
        if !(200..300).contains(&code) {
            let message = data.get("message").and_then(Value::as_str).unwrap_or_default().to_string();
            return Err(CallError::Refused { code, message });
        }
        serde_json::from_value(data).map_err(|e| CallError::Lost(format!("{}: {e}", R::MSG)))
    }

    /// Tells the core which UI this is, as the stock UI does after every sign-in.
    pub async fn announce_ui(&self, version: &str) -> Result<(), CallError> {
        self.call(&SetUiInfo { version: version.to_string(), entity_types: Vec::new() }).await.map(drop)
    }
}

/// `version`: what the remote is.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct GetVersion {}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct VersionInfo {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub device_name: String,
    #[serde(default)]
    pub hostname: String,
    /// The remote's address on the network.
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub api: String,
    #[serde(default)]
    pub core: String,
    #[serde(default)]
    pub ui: String,
    #[serde(default)]
    pub os: String,
}

impl Request for GetVersion {
    const MSG: &'static str = messages::VERSION;
    type Reply = VersionInfo;
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SetUiInfo {
    pub version: String,
    pub entity_types: Vec<String>,
}

impl Request for SetUiInfo {
    const MSG: &'static str = messages::SET_UI_INFO;
    type Reply = Value;
}

/// A text in each language the release notes are written in, by language code.
pub type LangMap = BTreeMap<String, String>;

/// The text in `lang`, else English, else whichever comes first.
pub fn localized<'a>(texts: &'a LangMap, lang: &str) -> Option<&'a str> {
    [lang, "en"].iter().find_map(|l| texts.get(*l)).or_else(|| texts.values().next()).map(String::as_str)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckSystemUpdate {
    pub force_update: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DownloadState {
    Pending,
    Downloading,
    Downloaded,
    Error,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AvailableUpdate {
    pub id: String,
    #[serde(default)]
    pub title: LangMap,
    #[serde(default)]
    pub description: LangMap,
    pub version: String,
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub release_date: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub release_notes_url: Option<String>,
    #[serde(default = "pending")]
    pub download: DownloadState,
}

fn pending() -> DownloadState {
    DownloadState::Pending
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct SystemUpdateInfo {
    #[serde(default)]
    pub update_in_progress: bool,
    #[serde(default)]
    pub last_check_date: Option<String>,
    #[serde(default)]
    pub installed_version: String,
    #[serde(default)]
    pub available: Vec<AvailableUpdate>,
}

impl Request for CheckSystemUpdate {
    const MSG: &'static str = messages::CHECK_SYSTEM_UPDATE;
    type Reply = SystemUpdateInfo;
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UpdateSystem {
    pub update_id: String,
}

/// What `update_system` began: fetching the update, or installing one already fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UpdateStart {
    Download,
    Start,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct UpdateSystemResult {
    pub state: UpdateStart,
}

impl Request for UpdateSystem {
    const MSG: &'static str = messages::UPDATE_SYSTEM;
    type Reply = UpdateSystemResult;
}

/// Why `update_system` did not begin, by the codes the spec lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateRefusal {
    /// 503: the battery is under half.
    BatteryLow,
    /// 409: an update is already running.
    Running,
    /// 422: the update is already downloaded.
    Downloaded,
    Other(String),
}

impl From<CallError> for UpdateRefusal {
    fn from(e: CallError) -> Self {
        match e {
            CallError::Refused { code: 503, .. } => Self::BatteryLow,
            CallError::Refused { code: 409, .. } => Self::Running,
            CallError::Refused { code: 422, .. } => Self::Downloaded,
            other => Self::Other(other.to_string()),
        }
    }
}

impl Core {
    pub async fn update_system(&self, update_id: &str) -> Result<UpdateStart, UpdateRefusal> {
        Ok(self.call(&UpdateSystem { update_id: update_id.to_string() }).await?.state)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UpdateEventType {
    Start,
    Progress,
    Stop,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UpdateRunState {
    Idle,
    Start,
    Run,
    Success,
    Failure,
    Download,
    Done,
    SubProcess,
    Progress,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct UpdateProgress {
    pub state: UpdateRunState,
    #[serde(default)]
    pub download_percent: Option<u8>,
    #[serde(default)]
    pub total_steps: Option<u32>,
    #[serde(default)]
    pub current_step: Option<u32>,
    #[serde(default)]
    pub current_percent: Option<u8>,
}

/// The event `software_update`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SoftwareUpdate {
    pub event_type: UpdateEventType,
    #[serde(default)]
    pub update_id: String,
    #[serde(default)]
    pub progress: Option<UpdateProgress>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct GetApiAccess {}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct WebConfiguratorAccess {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub valid_to: Option<String>,
}

/// Who may use the REST API; the PIN is never returned.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct ApiAccess {
    #[serde(default)]
    pub web_configurator: WebConfiguratorAccess,
}

impl Request for GetApiAccess {
    const MSG: &'static str = messages::GET_API_ACCESS;
    type Reply = ApiAccess;
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WebConfiguratorSetting {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SetApiAccess {
    pub web_configurator: WebConfiguratorSetting,
}

impl SetApiAccess {
    pub fn on(pin: &str) -> Self {
        Self { web_configurator: WebConfiguratorSetting { enabled: true, pin: Some(pin.to_string()) } }
    }

    pub fn off() -> Self {
        Self { web_configurator: WebConfiguratorSetting { enabled: false, pin: None } }
    }
}

impl Request for SetApiAccess {
    const MSG: &'static str = messages::SET_API_ACCESS;
    type Reply = Value;
}

impl Core {
    pub async fn api_access(&self) -> Result<ApiAccess, CallError> {
        self.call(&GetApiAccess {}).await
    }

    /// Turns the web configurator on with `pin`.
    pub async fn enable_web_configurator(&self, pin: &str) -> Result<(), CallError> {
        self.call(&SetApiAccess::on(pin)).await.map(drop)
    }

    pub async fn disable_web_configurator(&self) -> Result<(), CallError> {
        self.call(&SetApiAccess::off()).await.map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn requests_serialize_as_the_spec_shows() {
        assert_eq!(serde_json::to_value(GetVersion {}).unwrap(), json!({}));
        assert_eq!(serde_json::to_value(CheckSystemUpdate { force_update: true }).unwrap(), json!({"force_update": true}));
        assert_eq!(serde_json::to_value(SetApiAccess::on("1234")).unwrap(), json!({"web_configurator": {"enabled": true, "pin": "1234"}}));
        assert_eq!(serde_json::to_value(SetApiAccess::off()).unwrap(), json!({"web_configurator": {"enabled": false}}));
    }

    #[test]
    fn language_maps_prefer_the_asked_language_then_english() {
        let texts: LangMap = [("de", "Neu"), ("en", "New")].into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        assert_eq!(localized(&texts, "de"), Some("Neu"));
        assert_eq!(localized(&texts, "fr"), Some("New"));
        assert_eq!(localized(&LangMap::new(), "en"), None);
    }

    #[test]
    fn refusals_map_by_code() {
        let refused = |code| CallError::Refused { code, message: String::new() };
        assert_eq!(UpdateRefusal::from(refused(503)), UpdateRefusal::BatteryLow);
        assert_eq!(UpdateRefusal::from(refused(409)), UpdateRefusal::Running);
        assert_eq!(UpdateRefusal::from(refused(422)), UpdateRefusal::Downloaded);
        assert!(matches!(UpdateRefusal::from(refused(500)), UpdateRefusal::Other(_)));
    }

    #[test]
    fn software_update_events_parse() {
        let e = crate::Event {
            msg: messages::SOFTWARE_UPDATE.into(),
            cat: None,
            data: json!({"event_type": "PROGRESS", "update_id": "u1", "progress": {"state": "RUN", "total_steps": 4, "current_step": 2, "current_percent": 62}}),
        };
        let crate::CoreEvent::SoftwareUpdate(u) = e.typed() else { panic!("not an update event") };
        assert_eq!(u.event_type, UpdateEventType::Progress);
        let p = u.progress.unwrap();
        assert_eq!((p.state, p.current_step, p.total_steps, p.current_percent), (UpdateRunState::Run, Some(2), Some(4), Some(62)));
    }
}
