//! One driver's setup, as `modules.json` holds it: its type and its fields.

use crate::api::{Field, FieldKind};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriverEntry {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(flatten)]
    pub fields: Map<String, Value>,
}

impl DriverEntry {
    pub fn new(kind: &str) -> Self {
        Self { kind: kind.into(), fields: Map::new() }
    }

    /// A field as text, whether it was written as a string or a number.
    pub fn text(&self, key: &str) -> Option<String> {
        match self.fields.get(key)? {
            Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }

    pub fn flag(&self, key: &str) -> bool {
        self.fields.get(key) == Some(&Value::Bool(true))
    }

    /// Off is kept as nothing, so an untouched entry and one switched back off read the same.
    pub fn set_flag(&mut self, field: &Field, on: bool) {
        if on {
            self.fields.insert(field.key.to_string(), Value::Bool(true));
        } else {
            self.fields.remove(field.key);
        }
    }

    /// Sets a field as typed in: trimmed, a port kept as a number, a toggle as a bool, and empty clears it.
    pub fn set(&mut self, field: &Field, value: &str) {
        let value = value.trim();
        if field.kind == FieldKind::Toggle {
            self.set_flag(field, value.parse().unwrap_or(false));
        } else if value.is_empty() {
            self.fields.remove(field.key);
        } else if let Some(port) = value.parse::<u16>().ok().filter(|_| field.kind == FieldKind::Port) {
            self.fields.insert(field.key.to_string(), Value::from(port));
        } else {
            self.fields.insert(field.key.to_string(), Value::String(value.to_string()));
        }
    }

    /// A field's default as it is stored.
    pub fn set_default(&mut self, field: &Field) {
        if let Some(text) = field.default {
            self.set(field, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports_are_numbers_and_empty_clears() {
        let port = Field { key: "port", label: "Port", kind: FieldKind::Port, required: false, default: None };
        let host = Field { key: "host", label: "Host", kind: FieldKind::Text, required: true, default: None };
        let mut e = DriverEntry::new("x");
        e.set(&port, " 8081 ");
        e.set(&host, " theater.local ");
        assert_eq!(e.fields["port"], Value::from(8081));
        assert_eq!(e.text("port").as_deref(), Some("8081"));
        assert_eq!(e.fields["host"], Value::from("theater.local"));
        e.set(&host, "");
        assert!(!e.fields.contains_key("host"));
    }

    #[test]
    fn a_toggle_is_a_bool_and_off_is_nothing() {
        let tls = Field { key: "self_signed", label: "Self-signed", kind: FieldKind::Toggle, required: false, default: Some("true") };
        let mut e = DriverEntry::new("x");
        assert!(!e.flag("self_signed"));
        e.set_default(&tls);
        assert_eq!(e.fields["self_signed"], Value::Bool(true));
        assert!(e.flag("self_signed"));
        e.set_flag(&tls, false);
        assert!(!e.fields.contains_key("self_signed"));
    }
}
