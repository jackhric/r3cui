//! Every driver type r3cui knows. The only place that names them.

use crate::api::{Driver, Spec};
use crate::config::DriverEntry;
use serde_json::{Map, Value};
use std::sync::Arc;

pub type Start = Box<dyn Fn(&DriverEntry) -> Arc<dyn Driver> + Send + Sync>;

pub struct Registry {
    types: Vec<(&'static Spec, Start)>,
}

impl Registry {
    pub fn new() -> Self {
        Self { types: Vec::new() }
    }

    pub fn builtin() -> Self {
        Self::new()
            .with(&crate::homeassistant::SPEC, Box::new(|e| Arc::new(crate::homeassistant::HomeAssistant::start(e))))
            .with(&crate::kodi::SPEC, Box::new(|e| Arc::new(crate::kodi::Kodi::start(e))))
    }

    pub fn with(mut self, spec: &'static Spec, start: Start) -> Self {
        self.types.push((spec, start));
        self
    }

    pub fn specs(&self) -> impl Iterator<Item = &'static Spec> + '_ {
        self.types.iter().map(|(s, _)| *s)
    }

    pub fn spec(&self, kind: &str) -> Option<&'static Spec> {
        self.specs().find(|s| s.id == kind)
    }

    pub fn start(&self, entry: &DriverEntry) -> Option<Arc<dyn Driver>> {
        self.types.iter().find(|(s, _)| s.id == entry.kind).map(|(_, start)| start(entry))
    }

    /// A new driver's fields, from its spec's defaults.
    pub fn defaults(&self, kind: &str) -> Map<String, Value> {
        let Some(spec) = self.spec(kind) else { return Map::new() };
        spec.fields.iter().filter_map(|f| Some((f.key.to_string(), default_value(f.key, f.default?)))).collect()
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::builtin()
    }
}

fn default_value(key: &str, text: &str) -> Value {
    match text.parse::<u16>() {
        Ok(n) if key.ends_with("port") => Value::from(n),
        _ => Value::from(text),
    }
}
