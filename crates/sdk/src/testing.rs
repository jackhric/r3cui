//! Helpers for an integration's own tests.

use crate::api::Spec;
use crate::DriverEntry;
use std::time::Duration;
use tokio::sync::watch;

/// Waits up to five seconds for a watched value to satisfy `ok`, and returns it.
pub async fn until<T: Clone>(rx: &mut watch::Receiver<T>, ok: impl Fn(&T) -> bool) -> T {
    tokio::time::timeout(Duration::from_secs(5), rx.wait_for(|v| ok(v))).await.expect("in time").expect("open").clone()
}

/// The setup of one driver of type `spec`, as `modules.json` would hold it, with only `fields` set.
pub fn entry(spec: &Spec, fields: &[(&str, &str)]) -> DriverEntry {
    let mut e = DriverEntry::new(spec.id);
    for (key, value) in fields {
        let field = spec.field(key).unwrap_or_else(|| panic!("{} has no field {key}", spec.id));
        e.set(field, value);
    }
    e
}
