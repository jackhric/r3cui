//! What the client knows of Home Assistant, for the UI to show: plain data, published as one value.

use crate::error::Error;
use crate::ids::EntityId;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Clone, Debug, Default)]
pub struct HaState {
    pub link: Link,
    /// "2024.11.3", once connected.
    pub version: Option<String>,
    /// The watched entities, as of the last subscription.
    pub entities: BTreeMap<EntityId, EntityState>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Link {
    #[default]
    Unconfigured,
    Connecting,
    Up,
    Down { error: Error, retry_in: Duration },
    /// The token was refused; it is retried only rarely.
    Rejected(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntityState {
    pub state: String,
    pub attributes: Map<String, Value>,
}

impl EntityState {
    pub fn new(state: impl Into<String>, attributes: Map<String, Value>) -> Self {
        Self { state: state.into(), attributes }
    }

    pub fn is_available(&self) -> bool {
        !matches!(self.state.as_str(), "unavailable" | "unknown")
    }

    pub fn attr(&self, key: &str) -> Option<&Value> {
        self.attributes.get(key)
    }

    pub fn attr_str(&self, key: &str) -> Option<&str> {
        self.attr(key)?.as_str()
    }

    pub fn attr_f64(&self, key: &str) -> Option<f64> {
        self.attr(key)?.as_f64()
    }

    pub fn attr_bool(&self, key: &str) -> Option<bool> {
        self.attr(key)?.as_bool()
    }

    pub fn friendly_name(&self) -> Option<&str> {
        self.attr_str("friendly_name")
    }

    /// A sensor's value; `None` when it is unavailable or not a number.
    pub fn number(&self) -> Option<f64> {
        self.state.parse().ok().filter(|n: &f64| n.is_finite())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_and_numbers() {
        let s = |v: &str| EntityState::new(v, Map::new());
        assert!(s("on").is_available());
        assert!(!s("unavailable").is_available());
        assert!(!s("unknown").is_available());
        assert_eq!(s("21.5").number(), Some(21.5));
        assert_eq!(s("unavailable").number(), None);
        assert_eq!(s("NaN").number(), None);
    }
}
