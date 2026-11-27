//! What the web configurator shows and changes (`docs/web-configurator.md`): every integration's form laid out from its
//! `Spec`, global control and the inputs, as JSON; and the changes a browser asks for, as a new `Modules`. Pure: the
//! session answers the server with these.

use crate::app::integrations::{addable, global_driver, name_of, power_providers, role_kind, set_field, short, slot_drivers, spec_of, GLOBAL_ROLES};
use crate::host::config::{Binding, Modules, Role, Slot};
use crate::host::known::KnownTargets;
use crate::host::Host;
use r3cui_sdk::api::{Driver, FieldKind, Health, Paired, Target, TargetKind};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::Arc;

/// What the server asks of the session, which holds the drivers.
#[derive(Debug)]
pub(crate) enum Call {
    Config,
    Add(String),
    Update(String, Map<String, Value>),
    Remove(String),
    Bind(RoleName, Option<String>, String),
    Input(String, InputPatch),
    Paired(String, Paired),
    /// The driver itself, to ask for its targets or to pair, which take a while.
    Driver(String),
    /// What a driver answered for its targets, kept for the remote's own pickers.
    Targets { driver: String, kind: TargetKind, targets: Vec<Target> },
}

pub(crate) enum Answer {
    Config(Config),
    Driver(Arc<dyn Driver>),
    Done,
    Failed(String),
}

/// Answers `call`: a change is returned to apply, then the config as it stands after it.
pub(crate) fn change(host: &Host, call: &Call) -> Option<Result<Modules, String>> {
    Some(match call {
        Call::Add(kind) => add(host, kind),
        Call::Update(id, fields) => update(host, id, fields),
        Call::Remove(id) => remove(host, id),
        Call::Bind(role, driver, target) => bind(host, *role, driver.clone(), target.clone()),
        Call::Input(id, patch) => input(host, id, patch),
        Call::Paired(id, fields) => paired(host, id, fields.clone()),
        Call::Config | Call::Driver(_) | Call::Targets { .. } => return None,
    })
}

#[derive(Debug, Serialize)]
pub(crate) struct Config {
    remote: Remote,
    integrations: Vec<Integration>,
    addable: Vec<Kind>,
    global: Global,
    inputs: Vec<InputView>,
}

#[derive(Debug, Serialize)]
struct Remote {
    version: String,
}

#[derive(Debug, Serialize)]
struct Integration {
    id: String,
    kind: String,
    name: String,
    icon: &'static str,
    status: StatusView,
    fields: Vec<FieldView>,
    pairing: Option<PairingView>,
}

#[derive(Debug, Serialize)]
struct StatusView {
    health: &'static str,
    text: String,
}

#[derive(Debug, Serialize)]
struct FieldView {
    key: &'static str,
    label: &'static str,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_kind: Option<TargetKind>,
    required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    value_name: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    set: Option<bool>,
}

#[derive(Debug, Serialize)]
struct PairingView {
    code_len: usize,
}

#[derive(Debug, Serialize)]
struct Kind {
    kind: &'static str,
    name: &'static str,
    icon: &'static str,
}

#[derive(Debug, Serialize)]
struct Global {
    driver: Option<String>,
    roles: Vec<RoleView>,
}

#[derive(Debug, Serialize)]
struct RoleView {
    role: RoleName,
    label: &'static str,
    target_kind: TargetKind,
    bound: Option<Bound>,
    providers: Vec<Choice>,
}

#[derive(Debug, Serialize)]
struct Bound {
    driver: String,
    target: String,
    name: String,
}

#[derive(Debug, Serialize)]
struct Choice {
    driver: String,
    name: String,
}

#[derive(Debug, Serialize)]
struct InputView {
    id: String,
    name: String,
    hidden: bool,
    remote: Option<String>,
    ui: Option<String>,
    remote_choices: Vec<Choice>,
    ui_choices: Vec<Choice>,
}

/// A global role as the API names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RoleName {
    Volume,
    Source,
    Power,
    Voice,
}

impl RoleName {
    fn of(role: Role) -> Self {
        match role {
            Role::Volume => Self::Volume,
            Role::Source => Self::Source,
            Role::Power => Self::Power,
            Role::Voice => Self::Voice,
        }
    }

    fn role(self) -> Role {
        match self {
            Self::Volume => Role::Volume,
            Self::Source => Role::Source,
            Self::Power => Role::Power,
            Self::Voice => Role::Voice,
        }
    }

    pub(crate) fn parse(text: &str) -> Option<Self> {
        serde_json::from_value(Value::String(text.to_string())).ok()
    }
}

/// What a browser may change about an input; what it leaves out stays.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct InputPatch {
    name: Option<String>,
    hidden: Option<bool>,
    #[serde(default, deserialize_with = "some")]
    remote: Option<Option<String>>,
    #[serde(default, deserialize_with = "some")]
    ui: Option<Option<String>>,
}

/// Tells a field given as `null` (clear it) from one left out (leave it).
fn some<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

fn health(h: Health) -> &'static str {
    match h {
        Health::Up => "up",
        Health::Connecting => "connecting",
        Health::Down => "down",
        Health::NotSetUp => "not_set_up",
        Health::NotPaired => "not_paired",
    }
}

fn kind_name(kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::Text => "text",
        FieldKind::Url => "url",
        FieldKind::Port => "port",
        FieldKind::Secret => "secret",
        FieldKind::Credential => "credential",
        FieldKind::Target(_) => "target",
        FieldKind::Toggle => "toggle",
    }
}

const ROLE_LABELS: [(Role, &str); 4] = [(Role::Volume, "Volume"), (Role::Source, "Inputs"), (Role::Power, "Power"), (Role::Voice, "Voice")];

fn role_label(role: Role) -> &'static str {
    ROLE_LABELS.iter().find(|(r, _)| *r == role).map_or("", |(_, l)| l)
}

fn binding(m: &Modules, role: Role) -> Option<&Binding> {
    match role {
        Role::Volume => m.global.volume.as_ref(),
        Role::Source => m.global.source.as_ref(),
        Role::Power => m.global.power.as_ref(),
        Role::Voice => m.global.voice.as_ref(),
    }
}

pub(crate) fn config(host: &Host, known: &KnownTargets, version: &str) -> Config {
    let m = host.modules();
    let statuses = host.statuses();
    let integrations = m
        .drivers
        .iter()
        .map(|(id, entry)| {
            let spec = spec_of(host, id);
            let status = statuses.iter().find(|(s, ..)| s == id).map(|(_, _, s)| s.clone());
            let fields = spec
                .into_iter()
                .flat_map(|s| s.visible_fields())
                .map(|f| {
                    let mut view = FieldView { key: f.key, label: f.label, kind: kind_name(f.kind), target_kind: None, required: f.required, value: None, value_name: None, set: None };
                    match f.kind {
                        FieldKind::Secret => view.set = Some(entry.text(f.key).is_some()),
                        FieldKind::Toggle => view.value = Some(Value::Bool(entry.flag(f.key))),
                        FieldKind::Target(kind) => {
                            let value = entry.text(f.key);
                            view.target_kind = Some(kind);
                            view.value_name = Some(value.as_deref().and_then(|v| known.name(id, v)).map(str::to_string));
                            view.value = Some(value.map_or(Value::Null, Value::String));
                        }
                        _ => view.value = Some(entry.text(f.key).map_or(Value::Null, Value::String)),
                    }
                    view
                })
                .collect();
            Integration {
                id: id.clone(),
                kind: entry.kind.clone(),
                name: name_of(host, id),
                icon: spec.map_or("", |s| s.icon),
                status: status.map_or(StatusView { health: health(Health::NotSetUp), text: "Unknown type".into() }, |s| StatusView { health: health(s.health), text: s.word().to_string() }),
                fields,
                pairing: spec.and_then(|s| s.pairing).map(|p| PairingView { code_len: p.code_len }),
            }
        })
        .collect();
    let choices = |ids: Vec<String>| ids.into_iter().map(|d| Choice { name: name_of(host, &d), driver: d }).collect::<Vec<_>>();
    let global_id = global_driver(host);
    let roles = GLOBAL_ROLES
        .iter()
        .map(|&role| RoleView {
            role: RoleName::of(role),
            label: role_label(role),
            target_kind: role_kind(role),
            bound: binding(m, role).map(|b| {
                let name = if global_id.as_deref() == Some(b.driver.as_str()) { known.name(&b.driver, &b.target).map_or_else(|| short(&b.target), str::to_string) } else { name_of(host, &b.driver) };
                Bound { driver: b.driver.clone(), target: b.target.clone(), name }
            }),
            providers: if role == Role::Power { choices(power_providers(host)) } else { Vec::new() },
        })
        .collect();
    let inputs = host
        .inputs()
        .into_iter()
        .map(|i| InputView { id: i.id, name: i.name, hidden: i.hidden, remote: i.remote, ui: i.ui, remote_choices: choices(slot_drivers(host, Slot::Remote)), ui_choices: choices(slot_drivers(host, Slot::Ui)) })
        .collect();
    Config {
        remote: Remote { version: version.to_string() },
        integrations,
        addable: addable(host).map(|s| Kind { kind: s.id, name: s.name, icon: s.icon }).collect(),
        global: Global { driver: global_id, roles },
        inputs,
    }
}

pub(crate) fn add(host: &Host, kind: &str) -> Result<Modules, String> {
    if !addable(host).any(|s| s.id == kind) {
        return Err(format!("{kind} cannot be added: it is unknown or already set up"));
    }
    let mut m = host.modules().clone();
    m.add_driver(kind, host.registry().defaults(kind));
    Ok(m)
}

pub(crate) fn remove(host: &Host, id: &str) -> Result<Modules, String> {
    known_driver(host, id)?;
    let mut m = host.modules().clone();
    m.remove_driver(id);
    Ok(m)
}

/// Sets the fields given; a secret left out stays, credentials are not the browser's to set.
pub(crate) fn update(host: &Host, id: &str, fields: &Map<String, Value>) -> Result<Modules, String> {
    let spec = known_driver(host, id)?;
    let mut m = host.modules().clone();
    for (key, value) in fields {
        let field = spec.field(key).filter(|f| f.kind != FieldKind::Credential).ok_or_else(|| format!("{} has no field {key}", spec.name))?;
        match (field.kind, value) {
            (FieldKind::Toggle, Value::Bool(on)) => m.set_flag(id, field, *on),
            (FieldKind::Toggle, _) => return Err(format!("{} is on or off", field.label)),
            (_, Value::Null) => m.set_field(id, field, ""),
            (FieldKind::Port, Value::Number(n)) => m.set_field(id, field, &n.to_string()),
            (FieldKind::Port, Value::String(s)) if !s.trim().is_empty() && s.trim().parse::<u16>().is_err() => return Err(format!("{} is a port: a number up to 65535", field.label)),
            (_, Value::String(s)) => m.set_field(id, field, s),
            _ => return Err(format!("{} takes text", field.label)),
        }
    }
    Ok(m)
}

pub(crate) fn bind(host: &Host, role: RoleName, driver: Option<String>, target: String) -> Result<Modules, String> {
    let role = role.role();
    let binding = match driver {
        None => None,
        Some(d) if global_driver(host).as_deref() == Some(d.as_str()) => Some(Binding { driver: d, target }),
        Some(d) if role == Role::Power && power_providers(host).contains(&d) => Some(Binding { target: d.clone(), driver: d }),
        Some(d) => return Err(format!("{} cannot answer {}", name_of(host, &d), role_label(role))),
    };
    let mut m = host.modules().clone();
    m.bind(role, binding);
    Ok(m)
}

pub(crate) fn input(host: &Host, id: &str, patch: &InputPatch) -> Result<Modules, String> {
    if !host.inputs().iter().any(|i| i.id == id) {
        return Err(format!("There is no input {id}"));
    }
    let mut m = host.modules().clone();
    if let Some(name) = &patch.name {
        m.rename(id, name);
    }
    if let Some(hidden) = patch.hidden {
        m.set_hidden(id, hidden);
    }
    for (slot, wanted) in [(Slot::Remote, &patch.remote), (Slot::Ui, &patch.ui)] {
        if let Some(driver) = wanted {
            if let Some(d) = driver.as_deref().filter(|d| !slot_drivers(host, slot).iter().any(|s| s == d)) {
                return Err(format!("{} cannot fill that slot", name_of(host, d)));
            }
            m.set_slot(id, slot, driver.clone());
        }
    }
    Ok(m)
}

/// Stores what a pairing wrote.
pub(crate) fn paired(host: &Host, id: &str, fields: Paired) -> Result<Modules, String> {
    known_driver(host, id)?;
    let mut m = host.modules().clone();
    for (key, value) in fields {
        set_field(&mut m, host, id, key, &value);
    }
    Ok(m)
}

fn known_driver(host: &Host, id: &str) -> Result<&'static r3cui_sdk::api::Spec, String> {
    spec_of(host, id).ok_or_else(|| format!("There is no integration {id}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testing::{Fakes, FAKE_HOST, FAKE_PAIRABLE_CREDENTIAL};
    use serde_json::json;

    fn host() -> (Host, Fakes) {
        let fakes = Fakes::default();
        let mut m = Modules::default();
        m.add_driver("fake-global", Default::default());
        m.add_driver("fake-pairable", Default::default());
        (Host::new(fakes.registry(), m), fakes)
    }

    fn json_of(host: &Host) -> Value {
        serde_json::to_value(config(host, &KnownTargets::default(), "1.2.3")).unwrap()
    }

    #[tokio::test]
    async fn the_forms_are_laid_out_from_each_spec() {
        let (host, _fakes) = host();
        let c = json_of(&host);
        assert_eq!(c["remote"]["version"], "1.2.3");
        let pairable = c["integrations"].as_array().unwrap().iter().find(|i| i["id"] == "fake-pairable").unwrap();
        assert_eq!(pairable["fields"], json!([{ "key": "host", "label": "Host", "kind": "text", "required": true, "value": null }]), "credentials stay out");
        assert_eq!(pairable["pairing"], json!({ "code_len": 6 }));
        assert_eq!(c["addable"][0]["kind"], "fake-player");
        assert_eq!(c["global"]["driver"], "fake-global");
        assert_eq!(c["global"]["roles"][3], json!({ "role": "voice", "label": "Voice", "target_kind": "pipeline", "bound": null, "providers": [] }));
        assert_eq!(c["global"]["roles"][2]["providers"], json!([{ "driver": "fake-pairable", "name": "Pairable" }]));
    }

    #[tokio::test]
    async fn fields_change_as_typed_and_credentials_are_not_the_browsers() {
        let (mut host, _fakes) = host();
        host.apply(update(&host, "fake-pairable", json!({ "host": " tv.local " }).as_object().unwrap()).unwrap());
        assert_eq!(host.modules().drivers["fake-pairable"].text(FAKE_HOST).as_deref(), Some("tv.local"));
        assert!(update(&host, "fake-pairable", json!({ FAKE_PAIRABLE_CREDENTIAL: "x" }).as_object().unwrap()).is_err());
        assert!(update(&host, "nobody", &Map::new()).is_err());
        host.apply(paired(&host, "fake-pairable", vec![(FAKE_PAIRABLE_CREDENTIAL, "secret".into())]).unwrap());
        assert_eq!(host.modules().drivers["fake-pairable"].text(FAKE_PAIRABLE_CREDENTIAL).as_deref(), Some("secret"));
    }

    #[tokio::test]
    async fn roles_bind_to_the_global_drivers_targets_or_a_power_provider() {
        let (mut host, _fakes) = host();
        host.apply(bind(&host, RoleName::Volume, Some("fake-global".into()), "media_player.amp".into()).unwrap());
        host.apply(bind(&host, RoleName::Power, Some("fake-pairable".into()), String::new()).unwrap());
        let m = host.modules();
        assert_eq!(m.global.volume, Some(Binding { driver: "fake-global".into(), target: "media_player.amp".into() }));
        assert_eq!(m.global.power, Some(Binding { driver: "fake-pairable".into(), target: "fake-pairable".into() }));
        assert!(bind(&host, RoleName::Volume, Some("fake-pairable".into()), "x".into()).is_err(), "not a global driver");
        assert_eq!(json_of(&host)["global"]["roles"][0]["bound"]["name"], "amp", "named as the remote names it");
        host.apply(bind(&host, RoleName::Volume, None, String::new()).unwrap());
        assert_eq!(host.modules().global.volume, None);
    }

    #[tokio::test]
    async fn adding_and_removing_an_integration() {
        let (mut host, _fakes) = host();
        host.apply(add(&host, "fake-player").unwrap());
        assert!(add(&host, "fake-player").is_err(), "one of each");
        assert!(json_of(&host)["addable"].as_array().unwrap().is_empty());
        host.apply(remove(&host, "fake-player").unwrap());
        assert!(!host.modules().drivers.contains_key("fake-player"));
    }

    #[test]
    fn an_input_patch_tells_null_from_left_out() {
        let p: InputPatch = serde_json::from_value(json!({ "remote": null, "name": "Kodi" })).unwrap();
        assert_eq!((p.remote, p.ui, p.name.as_deref()), (Some(None), None, Some("Kodi")));
        assert_eq!(RoleName::parse("source"), Some(RoleName::Source));
        assert_eq!(RoleName::parse("bass"), None);
    }
}
