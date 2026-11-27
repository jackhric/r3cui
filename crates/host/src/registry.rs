//! The driver types this build of r3cui can run: the integrations the app lists, and none named here.

use r3cui_sdk::api::{Driver, Spec};
use r3cui_sdk::{DriverEntry, Integration};
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

    pub fn of(integrations: &[Integration]) -> Self {
        integrations.iter().fold(Self::new(), |r, i| r.with(i.spec, Box::new(i.start)))
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
        let mut entry = DriverEntry::new(kind);
        spec.fields.iter().for_each(|f| entry.set_default(f));
        entry.fields
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}
