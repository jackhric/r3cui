//! The client's logic, with no IO: each event (the link changing, an entity event) updates `HaState`. The service
//! feeds frames in and publishes the state. Because it never waits on anything, every rule here is tested with
//! plain values.

use crate::error::Error;
use crate::ids::EntityId;
use crate::state::{EntityState, HaState, Link};
use crate::wire::Compressed;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Connecting,
    Authenticated { version: Option<String> },
    Rejected { message: String },
    Disconnected { error: Error, retry_in: Duration },
    /// A new subscription is starting, and its first event holds everything again.
    Resubscribing,
    Entities(Compressed),
}

#[derive(Debug, Default)]
pub struct Engine {
    pub state: HaState,
}

impl Engine {
    pub fn apply(&mut self, event: Event) {
        match event {
            Event::Connecting => self.state.link = Link::Connecting,
            Event::Authenticated { version } => {
                self.state.link = Link::Up;
                self.state.version = version;
            }
            Event::Rejected { message } => {
                self.state.link = Link::Rejected(message);
                self.state.entities.clear();
            }
            Event::Disconnected { error, retry_in } => {
                self.state.link = Link::Down { error, retry_in };
                // Stale values would pass for current ones.
                self.state.entities.clear();
            }
            Event::Resubscribing => self.state.entities.clear(),
            Event::Entities(c) => self.entities(c),
        }
    }

    fn entities(&mut self, c: Compressed) {
        let map = &mut self.state.entities;
        for (id, added) in c.a {
            map.insert(id, EntityState { state: added.s, attributes: added.a });
        }
        for (id, change) in c.c {
            let Some(entity) = map.get_mut(&id) else {
                tracing::debug!("homeassistant: change for unknown {id}");
                continue;
            };
            if let Some(plus) = change.plus {
                if let Some(s) = plus.s {
                    entity.state = s;
                }
                entity.attributes.extend(plus.a.into_iter().flatten());
            }
            for key in change.minus.into_iter().flat_map(|m| m.a) {
                entity.attributes.remove(&key);
            }
        }
        for id in c.r {
            map.remove(&id);
        }
    }

    pub fn entity(&self, id: &EntityId) -> Option<&EntityState> {
        self.state.entities.get(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const RECEIVER: &str = "media_player.theater_receiver";

    fn entities(v: serde_json::Value) -> Event {
        Event::Entities(serde_json::from_value(v).unwrap())
    }

    fn with_receiver() -> Engine {
        let mut e = Engine::default();
        e.apply(entities(json!({"a": {RECEIVER: {
            "s": "on",
            "a": {
                "source_list": ["ANDROID TV", "Bluetooth", "HEOS Music", "KODI", "NET", "PLAYSTATION 5", "Source", "XBOX 360"],
                "volume_level": 0.56, "is_volume_muted": false, "source": "KODI", "device_class": "receiver",
                "friendly_name": "Theater Receiver", "supported_features": 69004
            },
            "c": "01JD8M0000000000000000000", "lc": 1732500000.1, "lu": 1732500000.1
        }}})));
        e
    }

    #[test]
    fn the_first_event_adds_entities_whole() {
        let e = with_receiver();
        let r = e.entity(&RECEIVER.into()).unwrap();
        assert_eq!(r.state, "on");
        assert_eq!(r.attr_f64("volume_level"), Some(0.56));
        assert_eq!(r.attr_str("source"), Some("KODI"));
        assert_eq!(r.attributes.len(), 7);
    }

    #[test]
    fn a_change_touches_only_what_it_names() {
        let mut e = with_receiver();
        e.apply(entities(json!({"c": {RECEIVER: {"+": {"a": {"volume_level": 0.6}, "c": "01JD8M1", "lu": 1732500005.0}}}})));
        let r = e.entity(&RECEIVER.into()).unwrap();
        assert_eq!((r.state.as_str(), r.attr_f64("volume_level"), r.attr_str("source")), ("on", Some(0.6), Some("KODI")));
        e.apply(entities(json!({"c": {RECEIVER: {"+": {"s": "off"}}}})));
        assert_eq!(e.entity(&RECEIVER.into()).unwrap().state, "off");
    }

    #[test]
    fn a_change_can_drop_attributes() {
        let mut e = with_receiver();
        e.apply(entities(json!({"c": {RECEIVER: {"+": {"s": "off"}, "-": {"a": ["source", "volume_level"]}}}})));
        let r = e.entity(&RECEIVER.into()).unwrap();
        assert_eq!((r.state.as_str(), r.attr("source"), r.attr("volume_level")), ("off", None, None));
        assert_eq!(r.friendly_name(), Some("Theater Receiver"));
    }

    #[test]
    fn removed_entities_go_and_unknown_changes_are_ignored() {
        let mut e = with_receiver();
        e.apply(entities(json!({"c": {"light.ghost": {"+": {"s": "on"}}}})));
        assert_eq!(e.state.entities.len(), 1);
        e.apply(entities(json!({"r": [RECEIVER]})));
        assert!(e.state.entities.is_empty());
    }

    #[test]
    fn one_event_can_carry_all_three() {
        let mut e = with_receiver();
        e.apply(entities(json!({
            "a": {"light.desk": {"s": "on", "a": {"brightness": 200}}},
            "c": {RECEIVER: {"+": {"s": "standby"}}},
            "r": ["light.nope"]
        })));
        assert_eq!(e.state.entities.len(), 2);
        assert_eq!(e.entity(&RECEIVER.into()).unwrap().state, "standby");
    }

    #[test]
    fn link_follows_the_connection() {
        let mut e = with_receiver();
        e.apply(Event::Authenticated { version: Some("2024.11.3".into()) });
        assert_eq!((e.state.link.clone(), e.state.version.as_deref()), (Link::Up, Some("2024.11.3")));
        let error = Error::Closed;
        e.apply(Event::Disconnected { error: error.clone(), retry_in: Duration::from_secs(1) });
        assert_eq!(e.state.link, Link::Down { error, retry_in: Duration::from_secs(1) });
        assert!(e.state.entities.is_empty(), "no stale values while away");
        e.apply(Event::Connecting);
        assert_eq!(e.state.link, Link::Connecting);
        e.apply(Event::Rejected { message: "Invalid access token or password".into() });
        assert_eq!(e.state.link, Link::Rejected("Invalid access token or password".into()));
    }

    #[test]
    fn resubscribing_starts_from_nothing() {
        let mut e = with_receiver();
        e.apply(Event::Resubscribing);
        assert!(e.state.entities.is_empty());
    }
}
