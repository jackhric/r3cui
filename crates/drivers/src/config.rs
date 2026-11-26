//! `modules.json`: the drivers set up, what global control points at, and the user's changes to the input list.
//! Every edit the Integrations pages make is a method here, so the rules (removing a driver clears its slots, an
//! input moved keeps the rest in order) are tested without a screen.

use crate::api::FieldKind;
use crate::kodi;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Modules {
    /// Instance id → its driver type and settings.
    pub drivers: BTreeMap<String, DriverEntry>,
    pub global: Global,
    /// The user's order; sources not listed follow in the order the source reports them.
    pub inputs: Vec<InputEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriverEntry {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(flatten)]
    pub fields: Map<String, Value>,
}

impl DriverEntry {
    /// A field as text, whether it was written as a string or a number.
    pub fn text(&self, key: &str) -> Option<String> {
        match self.fields.get(key)? {
            Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Global {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<Binding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Binding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub driver: String,
    pub target: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InputEntry {
    /// The source's name as it reports it: Home Assistant gives inputs no other id.
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui: Option<String>,
    /// Placed by the user rather than left in the source's order.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub placed: bool,
}

impl InputEntry {
    fn is_empty(&self) -> bool {
        self.name.is_none() && !self.hidden && self.remote.is_none() && self.ui.is_none() && !self.placed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Remote,
    Ui,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Volume,
    Source,
}

/// The old hardwired input Kodi was tied to, before inputs were configurable.
pub const LEGACY_KODI_INPUT: &str = "KODI";

/// What `settings.json` held before drivers were set up in `modules.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Legacy {
    pub kodi_host: Option<String>,
    pub kodi_http_port: Option<u16>,
    pub kodi_ws_port: Option<u16>,
    pub hidden_inputs: Vec<String>,
}

impl Modules {
    /// A first `modules.json` from the old settings: Kodi in the remote and UI slots of its input (and of Default,
    /// for a setup with no source yet), and the hidden inputs. `None` when the old settings held neither.
    pub fn from_legacy(old: &Legacy) -> Option<Self> {
        if old.kodi_host.is_none() && old.hidden_inputs.is_empty() {
            return None;
        }
        let mut m = Self::default();
        if let Some(host) = &old.kodi_host {
            let id = kodi::SPEC.id;
            m.add_driver(id, Default::default());
            let port = |p: Option<u16>| p.map(|p| p.to_string()).unwrap_or_default();
            for (key, value) in [(kodi::field::HOST, host.clone()), (kodi::field::HTTP_PORT, port(old.kodi_http_port)), (kodi::field::WS_PORT, port(old.kodi_ws_port))] {
                m.set_field(id, key, &value);
            }
            for input in [LEGACY_KODI_INPUT, crate::inputs::DEFAULT_INPUT] {
                m.set_slot(input, Slot::Remote, Some(id.into()));
                m.set_slot(input, Slot::Ui, Some(id.into()));
            }
        }
        for h in &old.hidden_inputs {
            m.set_hidden(h, true);
        }
        Some(m)
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| e.to_string())
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("modules serialise") + "\n"
    }

    pub fn input(&self, id: &str) -> Option<&InputEntry> {
        self.inputs.iter().find(|i| i.id == id)
    }

    fn input_mut(&mut self, id: &str) -> &mut InputEntry {
        match self.inputs.iter().position(|i| i.id == id) {
            Some(at) => &mut self.inputs[at],
            None => {
                self.inputs.push(InputEntry { id: id.to_string(), ..Default::default() });
                self.inputs.last_mut().unwrap()
            }
        }
    }

    fn tidy(&mut self) {
        self.inputs.retain(|i| !i.is_empty());
    }

    /// Adds a driver of `kind` with its defaults; the instance id is the type's, one per type for now.
    pub fn add_driver(&mut self, kind: &str, defaults: Map<String, Value>) -> String {
        self.drivers.entry(kind.to_string()).or_insert(DriverEntry { kind: kind.to_string(), fields: defaults });
        kind.to_string()
    }

    /// Removes a driver and everything that pointed at it.
    pub fn remove_driver(&mut self, id: &str) {
        self.drivers.remove(id);
        for role in [&mut self.global.volume, &mut self.global.source] {
            if role.as_ref().is_some_and(|b| b.driver == id) {
                *role = None;
            }
        }
        for input in &mut self.inputs {
            for slot in [&mut input.remote, &mut input.ui] {
                if slot.as_deref() == Some(id) {
                    *slot = None;
                }
            }
        }
        self.tidy();
    }

    /// An empty value clears the field.
    pub fn set_field(&mut self, driver: &str, key: &str, value: &str) {
        let Some(d) = self.drivers.get_mut(driver) else { return };
        let value = value.trim();
        if value.is_empty() {
            d.fields.remove(key);
        } else if let Some(port) = value.parse::<u16>().ok().filter(|_| crate::registry::field_kind(&d.kind, key) == Some(FieldKind::Port)) {
            d.fields.insert(key.to_string(), Value::from(port));
        } else {
            d.fields.insert(key.to_string(), Value::String(value.to_string()));
        }
    }

    pub fn bind(&mut self, role: Role, binding: Option<Binding>) {
        match role {
            Role::Volume => self.global.volume = binding,
            Role::Source => self.global.source = binding,
        }
    }

    pub fn set_slot(&mut self, input: &str, slot: Slot, driver: Option<String>) {
        let entry = self.input_mut(input);
        match slot {
            Slot::Remote => entry.remote = driver,
            Slot::Ui => entry.ui = driver,
        }
        self.tidy();
    }

    /// An empty name, or the source's own, removes the rename.
    pub fn rename(&mut self, input: &str, name: &str) {
        let name = name.trim();
        self.input_mut(input).name = (!name.is_empty() && name != input).then(|| name.to_string());
        self.tidy();
    }

    pub fn set_hidden(&mut self, input: &str, hidden: bool) {
        self.input_mut(input).hidden = hidden;
        self.tidy();
    }

    /// Moves an input one place among `order` (the list as shown, hidden ones included). Every input from the top
    /// down to the ones swapped is pinned in place, so the move holds however the source reorders its list later.
    pub fn move_input(&mut self, order: &[String], input: &str, by: isize) {
        let Some(from) = order.iter().position(|i| i == input) else { return };
        let to = from as isize + by;
        if to < 0 || to as usize >= order.len() {
            return;
        }
        let mut order = order.to_vec();
        order.swap(from, to as usize);
        let pinned = from.max(to as usize);
        let mut entries = Vec::with_capacity(self.inputs.len().max(order.len()));
        for (i, id) in order.iter().enumerate() {
            let mut entry = self.input(id).cloned().unwrap_or_else(|| InputEntry { id: id.clone(), ..Default::default() });
            entry.placed |= i <= pinned;
            entries.push(entry);
        }
        entries.extend(self.inputs.iter().filter(|e| !order.contains(&e.id)).cloned());
        self.inputs = entries;
        self.tidy();
    }

    /// Back to how the source reports it: its own name, shown, in its own place. The slots stay.
    pub fn reset_input(&mut self, input: &str) {
        let entry = self.input_mut(input);
        (entry.name, entry.hidden, entry.placed) = (None, false, false);
        self.tidy();
    }
}

/// Where `modules.json` lives: read and written in `dir`, with `seed` the copy shipped in the archive. Whichever was
/// written last wins: a new archive (with a token pasted into its copy) takes over, and changes made on the remote
/// since it was installed stay.
#[derive(Debug, Clone)]
pub struct Store {
    pub path: PathBuf,
    pub seed: Option<PathBuf>,
}

impl Store {
    pub fn new(dir: &Path, seed_dir: Option<&Path>) -> Self {
        Self { path: dir.join("modules.json"), seed: seed_dir.map(|d| d.join("modules.json")).filter(|s| *s != dir.join("modules.json")) }
    }

    /// The saved modules; `None` when there are none anywhere yet. A file that doesn't parse is an error, so a
    /// typo is never silently replaced by an empty setup.
    pub fn load(&self) -> Result<Option<Modules>, String> {
        let written = |p: &PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        // On a tie the remote's own copy wins: it comes last, and `max_by_key` keeps the last of equals.
        let newest = self.seed.iter().chain(std::iter::once(&self.path)).filter_map(|p| Some((written(p)?, p))).max_by_key(|(t, _)| *t);
        let Some((_, path)) = newest else { return Ok(None) };
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Modules::parse(&text).map(Some).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Written beside and renamed over, so a power loss leaves the old file or the new one, never half of one.
    pub fn save(&self, modules: &Modules) -> Result<(), String> {
        let tmp = self.path.with_extension("json.tmp");
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&tmp, modules.to_json()).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &self.path).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Modules {
        Modules::parse(
            r#"{
              "drivers": {
                "homeassistant": { "type": "homeassistant", "url": "http://ha:8123", "token": "t" },
                "kodi": { "type": "kodi", "host": "kodi", "ws_port": 9090 }
              },
              "global": {
                "volume": { "driver": "homeassistant", "target": "media_player.receiver" },
                "source": { "driver": "homeassistant", "target": "media_player.receiver" }
              },
              "inputs": [ { "id": "KODI", "remote": "kodi", "ui": "kodi" }, { "id": "NET", "hidden": true } ]
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn round_trips() {
        let m = sample();
        assert_eq!(Modules::parse(&m.to_json()).unwrap(), m);
        assert_eq!(m.drivers["kodi"].text("ws_port").as_deref(), Some("9090"));
        assert_eq!(m.drivers["kodi"].text("missing"), None);
        assert!(!m.to_json().contains("placed"), "defaults stay out of the file");
    }

    #[test]
    fn removing_a_driver_clears_what_pointed_at_it() {
        let mut m = sample();
        m.remove_driver("kodi");
        assert_eq!(m.input("KODI"), None, "nothing left on it");
        assert!(m.global.volume.is_some());
        m.remove_driver("homeassistant");
        assert_eq!(m.global, Global::default());
        assert!(m.input("NET").unwrap().hidden);
    }

    #[test]
    fn fields_are_set_and_cleared() {
        let mut m = sample();
        let kodi = kodi::SPEC.id;
        m.set_field(kodi, kodi::field::HTTP_PORT, "8081");
        m.set_field(kodi, kodi::field::HOST, " theater.local ");
        assert_eq!(m.drivers[kodi].fields[kodi::field::HTTP_PORT], Value::from(8081));
        assert_eq!(m.drivers[kodi].fields[kodi::field::HOST], Value::from("theater.local"));
        m.set_field(kodi, kodi::field::HOST, "");
        assert!(!m.drivers[kodi].fields.contains_key(kodi::field::HOST));
    }

    #[test]
    fn renaming_to_the_sources_own_name_is_no_rename() {
        let mut m = sample();
        m.rename("PLAYSTATION 5", "PS5");
        assert_eq!(m.input("PLAYSTATION 5").unwrap().name.as_deref(), Some("PS5"));
        m.rename("PLAYSTATION 5", "PLAYSTATION 5");
        assert_eq!(m.input("PLAYSTATION 5"), None);
        m.set_hidden("NET", false);
        assert_eq!(m.input("NET"), None);
    }

    #[test]
    fn moving_pins_the_inputs_above() {
        let mut m = sample();
        let order: Vec<String> = ["A", "KODI", "NET", "B"].map(String::from).to_vec();
        m.move_input(&order, "NET", -1);
        let ids: Vec<&str> = m.inputs.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["A", "NET", "KODI"]);
        assert!(m.inputs.iter().all(|i| i.placed));
        assert_eq!(m.input("KODI").unwrap().ui.as_deref(), Some("kodi"), "slots move with it");
        m.move_input(&order, "A", -1);
        assert_eq!(m.inputs.len(), 3, "can't move past the top");
        m.reset_input("A");
        assert_eq!(m.input("A"), None);
    }

    #[test]
    fn old_settings_fill_a_first_modules_json() {
        let old = Legacy { kodi_host: Some("theater".into()), kodi_ws_port: Some(9091), hidden_inputs: vec!["NET".into()], ..Default::default() };
        let m = Modules::from_legacy(&old).unwrap();
        let kodi = &m.drivers[kodi::SPEC.id];
        assert_eq!(kodi.text(kodi::field::HOST).as_deref(), Some("theater"));
        assert_eq!(kodi.text(kodi::field::WS_PORT).as_deref(), Some("9091"));
        assert_eq!(kodi.fields[kodi::field::WS_PORT], Value::from(9091), "ports are numbers");
        assert_eq!(kodi.text(kodi::field::HTTP_PORT), None);
        assert_eq!(m.input(LEGACY_KODI_INPUT).unwrap().ui.as_deref(), Some(kodi::SPEC.id));
        assert_eq!(m.input(crate::inputs::DEFAULT_INPUT).unwrap().remote.as_deref(), Some(kodi::SPEC.id));
        assert!(m.input("NET").unwrap().hidden);
        assert_eq!(Modules::from_legacy(&Legacy::default()), None);
    }

    #[test]
    fn the_store_seeds_then_saves() {
        let dir = std::env::temp_dir().join(format!("r3cui-store-{}", std::process::id()));
        let (data, seed) = (dir.join("data"), dir.join("config"));
        std::fs::create_dir_all(&seed).unwrap();
        let store = Store::new(&data, Some(&seed));
        assert_eq!(store.load().unwrap(), None);
        std::fs::write(seed.join("modules.json"), sample().to_json()).unwrap();
        assert_eq!(store.load().unwrap(), Some(sample()));
        let mut m = sample();
        m.remove_driver("kodi");
        std::thread::sleep(std::time::Duration::from_millis(20));
        store.save(&m).unwrap();
        assert_eq!(store.load().unwrap(), Some(m), "changes made here outlast the archive's copy");
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(seed.join("modules.json"), sample().to_json()).unwrap();
        assert_eq!(store.load().unwrap(), Some(sample()), "a newer archive's copy takes over");
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&store.path, "{ nope").unwrap();
        assert!(store.load().is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
