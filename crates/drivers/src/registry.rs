//! Every driver type r3cui knows. The only place that names them.

use crate::api::{Driver, FieldKind, Spec};
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
        BUILTIN.iter().fold(Self::new(), |r, &(spec, start)| r.with(spec, Box::new(start)))
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
        spec.fields.iter().filter_map(|f| Some((f.key.to_string(), default_value(f.kind, f.default?)))).collect()
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::builtin()
    }
}

fn default_value(kind: FieldKind, text: &str) -> Value {
    match text.parse::<u16>() {
        Ok(n) if kind == FieldKind::Port => Value::from(n),
        _ => Value::from(text),
    }
}

type StartFn = fn(&DriverEntry) -> Arc<dyn Driver>;

/// Every built-in driver type: the one list of them.
const BUILTIN: &[(&Spec, StartFn)] = &[
    (&crate::homeassistant::SPEC, |e| Arc::new(crate::homeassistant::HomeAssistant::start(e))),
    (&crate::kodi::SPEC, |e| Arc::new(crate::kodi::Kodi::start(e))),
    #[cfg(feature = "demo")]
    (&crate::demo::SPEC, |e| Arc::new(crate::demo::Demo::start(e))),
];

/// How a built-in driver type stores a field, if the type and field are known.
pub fn field_kind(driver_type: &str, key: &str) -> Option<FieldKind> {
    BUILTIN.iter().find(|(s, _)| s.id == driver_type)?.0.field(key).map(|f| f.kind)
}
