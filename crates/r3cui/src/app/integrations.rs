//! Settings → Integrations: the drivers set up, global control, and the inputs with their slots. Every screen is a
//! list drawn from the host and the driver specs, so a new driver needs no new screen. This is the logic alone: it
//! says what to show and what to do, and the session does it.

use crate::Dot;
use r3cui_sdk::api::{Field, FieldKind, Health, PairError, Paired, Spec, Target, TargetKind};
use host::{Binding, Host, Input, KnownTargets, Modules, Role, Slot};

#[derive(Debug, Clone, PartialEq)]
enum Screen {
    Root,
    Global,
    PickRole(Role),
    Driver(String),
    PickField { driver: String, key: &'static str },
    Remove(String),
    Add,
    Inputs,
    Input(String),
    PickSlot { input: String, slot: Slot },
    Pairing(String),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Row {
    /// A driver's mark, as SVG path data.
    pub icon: &'static str,
    pub label: String,
    pub detail: String,
    pub value: String,
    pub chevron: bool,
    pub checked: bool,
    pub dot: Dot,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Page {
    pub title: String,
    pub rows: Vec<Row>,
    pub focus: usize,
    /// How many screens deep: the page slides as this changes.
    pub depth: usize,
}

/// Where a pairing has got to.
#[derive(Debug, Clone, Default, PartialEq)]
enum Phase {
    /// The device is being contacted.
    #[default]
    Starting,
    /// The device shows its code; the keyboard is up, or was and was left.
    Waiting,
    Checking,
    Failed(String),
}

const CODE_TITLE: &str = "Pairing code";
const CODE_CAPTION: &str = "SHOWN ON THE DEVICE";
const CODE_WRONG: &str = "WRONG CODE, TRY AGAIN";

#[derive(Debug, PartialEq)]
pub(crate) enum Effect {
    Stay,
    Apply(Modules),
    /// Fetch what a target field can point at from `driver`; `targets_loaded` brings them.
    Load { driver: String, kind: TargetKind },
    Text { title: String, caption: String, text: String },
    /// The keyboard for a pairing code of `len` characters.
    Code { title: String, caption: String, len: usize },
    /// Start pairing `driver`, once `apply` (if any) has restarted it with what it needs.
    Pair { driver: String, apply: Option<Modules> },
    /// Give the code to the pairing under way.
    Finish { code: String },
    Leave,
}

#[derive(Debug, Clone, PartialEq)]
enum Editing {
    Field { driver: String, key: &'static str },
    Name(String),
    Code,
}

#[derive(Default)]
pub(crate) struct Integrations {
    /// The screens open, each with its focused row.
    stack: Vec<(Screen, usize)>,
    /// What the open picker offers; `None` while loading.
    targets: Option<Vec<Target>>,
    /// `targets` is the list this driver gave last time, not a fresh one.
    saved: Option<String>,
    known: KnownTargets,
    editing: Option<Editing>,
    phase: Phase,
}

/// The rows of Global control, in order.
const GLOBAL_ROLES: [Role; 4] = [Role::Volume, Role::Source, Role::Power, Role::Voice];

/// The first screen's rows: Global control, Inputs, then one per driver.
const ROOT_GLOBAL: usize = 0;
const ROOT_INPUTS: usize = 1;
const ROOT_DRIVERS: usize = 2;

fn row(label: impl Into<String>) -> Row {
    Row { label: label.into(), ..Default::default() }
}

fn spec_of(host: &Host, driver: &str) -> Option<&'static Spec> {
    host.registry().spec(&host.modules().drivers.get(driver)?.kind)
}

/// Sets one of a driver's fields as its spec types it; a key its spec doesn't have is left alone.
fn set_field(m: &mut Modules, host: &Host, driver: &str, key: &str, value: &str) {
    let spec = m.drivers.get(driver).and_then(|d| host.registry().spec(&d.kind));
    if let Some(field) = spec.and_then(|s| s.field(key)) {
        m.set_field(driver, field, value);
    }
}

fn icon_of(host: &Host, driver: &str) -> &'static str {
    spec_of(host, driver).map_or("", |s| s.icon)
}

fn name_of(host: &Host, driver: &str) -> String {
    spec_of(host, driver).map_or_else(|| driver.to_string(), |s| s.name.to_string())
}

/// The driver that can be global control. Home Assistant is the only one so far; one instance per type.
fn global_driver(host: &Host) -> Option<String> {
    host.modules().drivers.keys().find(|id| spec_of(host, id).is_some_and(|s| s.provides.global)).cloned()
}

/// The drivers that can be global power without being global control.
fn power_providers(host: &Host) -> Vec<String> {
    host.modules().drivers.keys().filter(|id| spec_of(host, id).is_some_and(|s| s.provides.power && !s.provides.global)).cloned().collect()
}

/// What a binding points at, as a person would say it: the target's name as its driver last gave it, or the driver
/// itself.
fn bound_name(host: &Host, known: &KnownTargets, b: &Binding) -> String {
    if spec_of(host, &b.driver).is_some_and(|s| s.provides.global) { target_name(known, &b.driver, &b.target) } else { name_of(host, &b.driver) }
}

fn target_name(known: &KnownTargets, driver: &str, id: &str) -> String {
    known.name(driver, id).map_or_else(|| short(id), str::to_string)
}

fn input_of(host: &Host, id: &str) -> Option<Input> {
    host.inputs().into_iter().find(|i| i.id == id)
}

impl Integrations {
    /// Opens on the first screen.
    pub(crate) fn open(&mut self) {
        (self.stack, self.targets, self.saved, self.editing, self.phase) = (vec![(Screen::Root, 0)], None, None, None, Phase::default());
    }

    /// Opens where an empty input list is fixed: global control's Inputs when nothing is set up as the source, the
    /// source's own screen while it has listed none, else the input list (every input is hidden).
    pub(crate) fn open_for_inputs(&mut self, host: &Host) {
        self.open();
        let m = host.modules();
        self.stack = match &m.global.source {
            None => vec![(Screen::Root, ROOT_GLOBAL), (Screen::Global, GLOBAL_ROLES.iter().position(|r| *r == Role::Source).unwrap_or(0))],
            Some(b) if host.inputs().is_empty() => {
                let at = ROOT_DRIVERS + m.drivers.keys().position(|id| *id == b.driver).unwrap_or(0);
                vec![(Screen::Root, at), (Screen::Driver(b.driver.clone()), 0)]
            }
            Some(_) => vec![(Screen::Root, ROOT_INPUTS), (Screen::Inputs, 0)],
        };
    }

    fn top(&self) -> &Screen {
        self.stack.last().map_or(&Screen::Root, |(s, _)| s)
    }

    fn push(&mut self, screen: Screen) {
        (self.targets, self.saved) = (None, None);
        self.stack.push((screen, 0));
    }

    pub(crate) fn with_known(known: KnownTargets) -> Self {
        Self { known, ..Self::default() }
    }

    pub(crate) fn known(&self) -> &KnownTargets {
        &self.known
    }

    /// A picker is fetching `driver`'s targets: meanwhile it shows the list from last time.
    pub(crate) fn loading(&mut self, driver: &str, kind: TargetKind) {
        if let Some(saved) = self.known.get(driver, kind) {
            (self.targets, self.saved) = (Some(saved.to_vec()), Some(driver.to_string()));
        }
    }

    /// What the driver answered. An empty answer keeps the list from last time on screen; a list is kept for next
    /// time. `true` when what is kept changed and should be saved.
    pub(crate) fn targets_loaded(&mut self, driver: &str, kind: TargetKind, targets: Vec<Target>) -> bool {
        if targets.is_empty() {
            self.targets.get_or_insert_with(Vec::new);
            return false;
        }
        (self.targets, self.saved) = (Some(targets.clone()), None);
        self.known.put(driver, kind, targets)
    }

    pub(crate) fn page(&self, host: &Host) -> Page {
        let m = host.modules();
        let focus = self.stack.last().map_or(0, |(_, f)| *f);
        let (title, rows) = match self.top() {
            Screen::Root => {
                let mut rows = vec![
                    Row { value: m.global.source.as_ref().or(m.global.volume.as_ref()).or(m.power_binding()).map_or("None".into(), |b| bound_name(host, &self.known, b)), chevron: true, ..row("Global control") },
                    Row { value: format!("{} shown", host.inputs().iter().filter(|i| !i.hidden).count()), chevron: true, ..row("Inputs") },
                ];
                let statuses = host.statuses();
                for id in m.drivers.keys() {
                    let status = statuses.iter().find(|(s, ..)| s == id).map(|(_, _, s)| s);
                    let dot = match status.map(|s| s.health) {
                        Some(Health::Up) => Dot::Up,
                        Some(Health::Down) => Dot::Down,
                        _ => Dot::Unknown,
                    };
                    let value = status.map_or("Unknown type".to_string(), |s| s.word().to_string());
                    rows.push(Row { icon: icon_of(host, id), value, chevron: true, dot, ..row(name_of(host, id)) });
                }
                if self.addable(host).next().is_some() {
                    rows.push(Row { chevron: true, ..row("Add integration") });
                }
                ("Integrations".to_string(), rows)
            }
            Screen::Global => {
                let target = |b: &Option<Binding>| b.as_ref().map_or("None".to_string(), |b| bound_name(host, &self.known, b));
                let rows = vec![
                    Row { value: target(&m.global.volume), chevron: true, detail: "Volume and mute".into(), ..row("Volume") },
                    Row { value: target(&m.global.source), chevron: true, detail: "The input list".into(), ..row("Inputs") },
                    Row { value: target(&m.power_binding().cloned()), chevron: true, detail: "On and off".into(), ..row("Power") },
                    Row { value: target(&m.global.voice), chevron: true, detail: "Where the voice key sends speech".into(), ..row("Voice") },
                ];
                ("Global control".to_string(), rows)
            }
            Screen::PickRole(role) => {
                let bound = match role {
                    Role::Volume => &m.global.volume,
                    Role::Source => &m.global.source,
                    Role::Power => &m.power_binding().cloned(),
                    Role::Voice => &m.global.voice,
                };
                let title = match role {
                    Role::Volume => "Volume",
                    Role::Source => "Inputs",
                    Role::Power => "Power",
                    Role::Voice => "Voice",
                };
                let providers = if *role == Role::Power { power_providers(host) } else { Vec::new() };
                let on_provider = bound.as_ref().is_some_and(|b| providers.contains(&b.driver));
                let global = global_driver(host);
                let mut rows = match &global {
                    Some(d) => self.target_rows(d, bound.as_ref().filter(|_| !on_provider).map(|b| b.target.as_str()), !on_provider && bound.is_none()),
                    None if providers.is_empty() => vec![row("Add Home Assistant first")],
                    None => vec![Row { checked: bound.is_none(), ..row("None") }],
                };
                rows.extend(providers.iter().map(|id| Row { icon: icon_of(host, id), checked: bound.as_ref().is_some_and(|b| b.driver == *id), ..row(name_of(host, id)) }));
                (self.offline_title(host, title, global.as_deref()), rows)
            }
            Screen::Driver(id) => {
                let entry = &m.drivers[id];
                let spec = spec_of(host, id);
                let mut rows: Vec<Row> = spec
                    .into_iter()
                    .flat_map(|s| s.visible_fields())
                    .map(|f| {
                        if f.kind == FieldKind::Toggle {
                            return Row { value: if entry.flag(f.key) { "On" } else { "Off" }.into(), ..row(f.label) };
                        }
                        let value = match (f.kind, entry.text(f.key)) {
                            (FieldKind::Secret, Some(_)) => "Set".to_string(),
                            (FieldKind::Secret, None) | (_, None) if f.required => "Not set".to_string(),
                            (_, None) => "None".to_string(),
                            (_, Some(v)) => v,
                        };
                        Row { value, chevron: true, ..row(f.label) }
                    })
                    .collect();
                if spec.is_some_and(|s| s.pairing.is_some()) {
                    let needed = host.statuses().iter().any(|(d, _, s)| d == id && s.health == Health::NotPaired);
                    rows.push(Row { value: if needed { "Needed".into() } else { String::new() }, chevron: true, detail: "Type the code the device shows".into(), ..row("Pair") });
                }
                rows.push(row(format!("Remove {}", name_of(host, id))));
                (name_of(host, id), rows)
            }
            Screen::Pairing(id) => {
                let name = name_of(host, id);
                let rows = match &self.phase {
                    Phase::Starting => vec![row(format!("Contacting {name}…"))],
                    Phase::Waiting => vec![Row { detail: "Shown on the device".into(), chevron: true, ..row("Enter the code") }],
                    Phase::Checking => vec![row("Checking the code…")],
                    Phase::Failed(why) => vec![Row { detail: why.clone(), ..row("Try again") }],
                };
                (format!("Pair {name}"), rows)
            }
            Screen::PickField { driver, key } => {
                let label = spec_of(host, driver).and_then(|s| s.fields.iter().find(|f| f.key == *key)).map_or(*key, |f| f.label);
                let current = m.drivers.get(driver).and_then(|d| d.text(key));
                (self.offline_title(host, label, Some(driver)), self.target_rows(driver, current.as_deref(), current.is_none()))
            }
            Screen::Remove(id) => (format!("Remove {}?", name_of(host, id)), vec![row(format!("Remove {}", name_of(host, id))), row("Cancel")]),
            Screen::Add => {
                let rows = self
                    .addable(host)
                    .map(|s| {
                        let jobs: Vec<&str> =
                            [(s.provides.global, "volume, inputs and power"), (s.provides.remote, "buttons"), (s.provides.ui, "screen"), (s.provides.power && !s.provides.global, "power")].into_iter().filter(|(y, _)| *y).map(|(_, j)| j).collect();
                        Row { icon: s.icon, detail: capitalise(&jobs.join(", ")), chevron: true, ..row(s.name) }
                    })
                    .collect();
                ("Add integration".to_string(), rows)
            }
            Screen::Inputs => {
                let rows = host
                    .inputs()
                    .into_iter()
                    .map(|i| Row {
                        icon: host.icon(&i).unwrap_or(""),
                        detail: if i.hidden { "Hidden".into() } else { String::new() },
                        value: i.ui.as_deref().map(|d| name_of(host, d)).unwrap_or_default(),
                        chevron: true,
                        dot: if i.changed { Dot::Changed } else { Dot::None },
                        ..row(i.name)
                    })
                    .collect::<Vec<_>>();
                let rows = if rows.is_empty() { vec![row("Waiting for the source's inputs")] } else { rows };
                ("Inputs".to_string(), rows)
            }
            Screen::Input(id) => match input_of(host, id) {
                Some(i) => {
                    let slot = |d: &Option<String>| d.as_deref().map_or("Nothing".to_string(), |d| name_of(host, d));
                    let renamed = i.name != i.id;
                    let mut rows = vec![
                        Row { value: i.name.clone(), chevron: true, dot: if renamed { Dot::Changed } else { Dot::None }, ..row("Name") },
                        Row { checked: !i.hidden, ..row("Show in the list") },
                        row("Move up"),
                        row("Move down"),
                        Row { value: slot(&i.remote), chevron: true, detail: "Where the buttons go".into(), ..row("Remote") },
                        Row { value: slot(&i.ui), chevron: true, detail: "What the screen shows".into(), ..row("Screen") },
                    ];
                    if i.changed {
                        rows.push(Row { detail: "Its own name, shown, in its own place".into(), ..row("Reset") });
                    }
                    (i.name, rows)
                }
                None => (id.clone(), vec![row("The source no longer lists it")]),
            },
            Screen::PickSlot { input, slot } => {
                let current = input_of(host, input).and_then(|i| if *slot == Slot::Remote { i.remote } else { i.ui });
                let mut rows = vec![Row { checked: current.is_none(), ..row("Nothing") }];
                for id in self.slot_drivers(host, *slot) {
                    rows.push(Row { icon: icon_of(host, &id), checked: current.as_deref() == Some(id.as_str()), ..row(name_of(host, &id)) });
                }
                (if *slot == Slot::Remote { "Remote" } else { "Screen" }.to_string(), rows)
            }
        };
        let focus = focus.min(rows.len().saturating_sub(1));
        Page { title, rows, focus, depth: self.stack.len() }
    }

    /// "None" (ticked when `none_checked`), then what the picker offers, the current one ticked.
    /// A picker's title, marked when it shows the list from last time because its driver is not connected.
    fn offline_title(&self, host: &Host, title: &str, driver: Option<&str>) -> String {
        let up = |d: &str| host.statuses().iter().any(|(id, _, s)| id == d && s.health == Health::Up);
        match driver {
            Some(d) if self.saved.as_deref() == Some(d) && !up(d) => format!("{title} · offline"),
            _ => title.to_string(),
        }
    }

    /// With nothing to list, what is bound still shows, by the name its driver last gave it.
    fn target_rows(&self, driver: &str, current: Option<&str>, none_checked: bool) -> Vec<Row> {
        let mut rows = vec![Row { checked: none_checked, ..row("None") }];
        let bound = current.map(|id| Row { checked: true, detail: id.to_string(), ..row(target_name(&self.known, driver, id)) });
        match &self.targets {
            None => rows.push(bound.unwrap_or_else(|| row("Loading…"))),
            Some(t) if t.is_empty() => rows.push(bound.unwrap_or_else(|| row("Nothing found"))),
            Some(t) => rows.extend(t.iter().map(|t| Row { detail: t.id.clone(), checked: current == Some(t.id.as_str()), ..row(t.name.clone()) })),
        }
        rows
    }

    fn addable<'a>(&self, host: &'a Host) -> impl Iterator<Item = &'static Spec> + 'a {
        host.registry().specs().filter(move |s| !host.modules().drivers.values().any(|d| d.kind == s.id))
    }

    fn slot_drivers(&self, host: &Host, slot: Slot) -> Vec<String> {
        host.modules()
            .drivers
            .keys()
            .filter(|id| spec_of(host, id).is_some_and(|s| if slot == Slot::Remote { s.provides.remote } else { s.provides.ui }))
            .cloned()
            .collect()
    }

    /// The target a picker row stands for: `None` for the first row, the row's target after; nothing while loading.
    fn picked_target(&self, row: usize) -> Option<Option<String>> {
        match row {
            0 => Some(None),
            n => self.targets.as_ref()?.get(n - 1).map(|t| Some(t.id.clone())),
        }
    }

    pub(crate) fn pick(&mut self, at: usize, host: &Host) -> Effect {
        if let Some((_, focus)) = self.stack.last_mut() {
            *focus = at;
        }
        let mut m = host.modules().clone();
        match self.top().clone() {
            Screen::Root => {
                let drivers: Vec<String> = host.modules().drivers.keys().cloned().collect();
                match at {
                    ROOT_GLOBAL => self.push(Screen::Global),
                    ROOT_INPUTS => self.push(Screen::Inputs),
                    n if n - ROOT_DRIVERS < drivers.len() => self.push(Screen::Driver(drivers[n - ROOT_DRIVERS].clone())),
                    _ if self.addable(host).next().is_some() => self.push(Screen::Add),
                    _ => {}
                }
                Effect::Stay
            }
            Screen::Global => {
                let role = GLOBAL_ROLES[at.min(GLOBAL_ROLES.len() - 1)];
                self.push(Screen::PickRole(role));
                let kind = if role == Role::Voice { TargetKind::Pipeline } else { TargetKind::MediaPlayer };
                match global_driver(host) {
                    Some(driver) => Effect::Load { driver, kind },
                    None => Effect::Stay,
                }
            }
            Screen::PickRole(role) => {
                let rows_before_providers = if global_driver(host).is_some() { self.targets.as_ref().map_or(1, |t| t.len().max(1)) } else { 0 };
                let provider = at.checked_sub(rows_before_providers + 1).and_then(|i| power_providers(host).get(i).cloned()).filter(|_| role == Role::Power);
                let binding = match (at, provider) {
                    (0, _) => None,
                    (_, Some(driver)) => Some(Binding { target: driver.clone(), driver }),
                    _ => {
                        let (Some(driver), Some(Some(target))) = (global_driver(host), self.picked_target(at)) else { return Effect::Stay };
                        Some(Binding { driver, target })
                    }
                };
                m.bind(role, binding);
                self.pop();
                Effect::Apply(m)
            }
            Screen::Driver(id) => {
                let spec = spec_of(host, &id);
                let fields: Vec<&Field> = spec.into_iter().flat_map(|s| s.visible_fields()).collect();
                if spec.is_some_and(|s| s.pairing.is_some()) && at == fields.len() {
                    return self.start_pairing(id, None);
                }
                match fields.get(at).copied() {
                    Some(f) => match f.kind {
                        FieldKind::Toggle => {
                            let on = m.drivers.get(&id).is_some_and(|d| d.flag(f.key));
                            m.set_flag(&id, f, !on);
                            Effect::Apply(m)
                        }
                        FieldKind::Target(kind) => {
                            self.push(Screen::PickField { driver: id.clone(), key: f.key });
                            Effect::Load { driver: id, kind }
                        }
                        kind => {
                            let current = m.drivers.get(&id).and_then(|d| d.text(f.key)).filter(|_| kind != FieldKind::Secret).unwrap_or_default();
                            self.editing = Some(Editing::Field { driver: id.clone(), key: f.key });
                            Effect::Text { title: f.label.into(), caption: name_of(host, &id).to_uppercase(), text: current }
                        }
                    },
                    None => {
                        self.push(Screen::Remove(id));
                        Effect::Stay
                    }
                }
            }
            Screen::PickField { driver, key } => {
                let Some(target) = self.picked_target(at) else { return Effect::Stay };
                set_field(&mut m, host, &driver, key, target.as_deref().unwrap_or(""));
                self.pop();
                Effect::Apply(m)
            }
            Screen::Remove(id) => {
                if at == 0 {
                    m.remove_driver(&id);
                    self.pop();
                    self.pop();
                    return Effect::Apply(m);
                }
                self.pop();
                Effect::Stay
            }
            Screen::Add => {
                let Some(spec) = self.addable(host).nth(at) else { return Effect::Stay };
                let id = m.add_driver(spec.id, host.registry().defaults(spec.id));
                self.pop();
                self.push(Screen::Driver(id));
                Effect::Apply(m)
            }
            Screen::Inputs => {
                if let Some(i) = host.inputs().get(at) {
                    self.push(Screen::Input(i.id.clone()));
                }
                Effect::Stay
            }
            Screen::Input(id) => {
                let Some(input) = input_of(host, &id) else { return Effect::Stay };
                let order: Vec<String> = host.inputs().into_iter().map(|i| i.id).collect();
                match at {
                    0 => {
                        self.editing = Some(Editing::Name(id.clone()));
                        return Effect::Text { title: "Name".into(), caption: input.id.clone(), text: input.name };
                    }
                    1 => m.set_hidden(&id, !input.hidden),
                    2 | 3 => {
                        let by = if at == 2 { -1 } else { 1 };
                        m.move_input(&order, &id, by);
                        // The list behind keeps its focus on the input that moved.
                        if let Some(at) = order.iter().position(|i| *i == id) {
                            let to = (at as isize + by).clamp(0, order.len() as isize - 1) as usize;
                            if let Some((_, focus)) = self.stack.iter_mut().rev().nth(1) {
                                *focus = to;
                            }
                        }
                    }
                    4 => {
                        self.push(Screen::PickSlot { input: id, slot: Slot::Remote });
                        return Effect::Stay;
                    }
                    5 => {
                        self.push(Screen::PickSlot { input: id, slot: Slot::Ui });
                        return Effect::Stay;
                    }
                    _ => m.reset_input(&id),
                }
                Effect::Apply(m)
            }
            Screen::Pairing(id) => match self.phase.clone() {
                Phase::Waiting => self.ask_code(&id, host, CODE_CAPTION),
                Phase::Failed(_) => self.start_pairing(id, None),
                Phase::Starting | Phase::Checking => Effect::Stay,
            },
            Screen::PickSlot { input, slot } => {
                let driver = match at {
                    0 => None,
                    n => match self.slot_drivers(host, slot).get(n - 1) {
                        Some(d) => Some(d.clone()),
                        None => return Effect::Stay,
                    },
                };
                m.set_slot(&input, slot, driver);
                self.pop();
                Effect::Apply(m)
            }
        }
    }

    /// The keyboard's Done. A pairable driver whose setup is complete, with nothing paired yet, goes on to pairing.
    pub(crate) fn text_entered(&mut self, text: &str, host: &Host) -> Effect {
        let mut m = host.modules().clone();
        match self.editing.take() {
            None => return Effect::Stay,
            Some(Editing::Code) => {
                self.phase = Phase::Checking;
                return Effect::Finish { code: text.trim().to_string() };
            }
            Some(Editing::Name(input)) => m.rename(&input, text),
            Some(Editing::Field { driver, key }) => {
                set_field(&mut m, host, &driver, key, text);
                if needs_pairing(&m, &driver, host) {
                    return self.start_pairing(driver, Some(m));
                }
            }
        }
        Effect::Apply(m)
    }

    fn start_pairing(&mut self, driver: String, apply: Option<Modules>) -> Effect {
        if self.stack.last().is_none_or(|(s, _)| !matches!(s, Screen::Pairing(_))) {
            self.push(Screen::Pairing(driver.clone()));
        }
        self.phase = Phase::Starting;
        Effect::Pair { driver, apply }
    }

    /// The device answered and shows its code, or it did not.
    pub(crate) fn pair_started(&mut self, started: Result<(), PairError>, host: &Host) -> Effect {
        let Screen::Pairing(driver) = self.top().clone() else { return Effect::Stay };
        match started {
            Ok(()) => self.ask_code(&driver, host, CODE_CAPTION),
            Err(e) => {
                self.phase = Phase::Failed(pair_failure(e));
                Effect::Stay
            }
        }
    }

    /// The pairing's verdict on the code: done, mistyped (the keyboard again), or failed.
    pub(crate) fn pair_checked(&mut self, result: Result<Paired, PairError>, host: &Host) -> Effect {
        let Screen::Pairing(driver) = self.top().clone() else { return Effect::Stay };
        match result {
            Ok(fields) => {
                let mut m = host.modules().clone();
                for (key, value) in fields {
                    set_field(&mut m, host, &driver, key, &value);
                }
                self.pop();
                Effect::Apply(m)
            }
            Err(PairError::WrongCode) => self.ask_code(&driver, host, CODE_WRONG),
            Err(e) => {
                self.phase = Phase::Failed(pair_failure(e));
                Effect::Stay
            }
        }
    }

    fn ask_code(&mut self, driver: &str, host: &Host, caption: &str) -> Effect {
        let Some(pairing) = spec_of(host, driver).and_then(|s| s.pairing) else { return Effect::Stay };
        self.phase = Phase::Waiting;
        self.editing = Some(Editing::Code);
        Effect::Code { title: CODE_TITLE.into(), caption: caption.into(), len: pairing.code_len }
    }

    fn pop(&mut self) {
        (self.targets, self.saved) = (None, None);
        self.stack.pop();
    }

    /// One screen back; `Leave` from the first.
    pub(crate) fn back(&mut self) -> Effect {
        self.pop();
        if self.stack.is_empty() { Effect::Leave } else { Effect::Stay }
    }
}

/// Whether `driver` can be paired, has all it needs to be, and has nothing paired yet.
fn needs_pairing(m: &Modules, driver: &str, host: &Host) -> bool {
    let (Some(entry), Some(spec)) = (m.drivers.get(driver), spec_of(host, driver)) else { return false };
    let has = |f: &Field| entry.text(f.key).is_some();
    spec.pairing.is_some() && spec.fields.iter().filter(|f| f.required).all(has) && spec.fields.iter().filter(|f| f.kind == FieldKind::Credential).all(|f| !has(f))
}

fn pair_failure(e: PairError) -> String {
    match e {
        PairError::WrongCode => "Wrong code".into(),
        PairError::Failed(why) => why,
    }
}

/// An entity id without its domain: `media_player.receiver` → `receiver`.
fn short(target: &str) -> String {
    target.split_once('.').map_or(target, |(_, name)| name).to_string()
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use r3cui_sdk::api::Sources;
    use r3cui_sdk::api::PairError;
    use host::testing::{Fakes, FAKE_PAIRABLE, FAKE_PAIRABLE_CREDENTIAL};

    fn setup() -> (Host, Fakes, Integrations) {
        let fakes = Fakes::default();
        let mut m = Modules::default();
        m.add_driver("fake-global", Default::default());
        m.bind(Role::Source, Some(Binding { driver: "fake-global".into(), target: "receiver".into() }));
        let host = Host::new(fakes.registry(), m);
        fakes.get("fake-global").set_sources(Sources { current: "TV".into(), list: vec!["TV".into(), "KODI".into()] });
        let mut ui = Integrations::default();
        ui.open();
        (host, fakes, ui)
    }

    fn labels(p: &Page) -> Vec<&str> {
        p.rows.iter().map(|r| r.label.as_str()).collect()
    }

    /// Picks the row labelled `label`, applying what it changes.
    fn tap(ui: &mut Integrations, host: &mut Host, label: &str) -> Effect {
        let at = ui.page(host).rows.iter().position(|r| r.label == label).unwrap_or_else(|| panic!("no {label:?} in {:?}", labels(&ui.page(host))));
        let effect = ui.pick(at, host);
        if let Effect::Apply(m) = &effect {
            host.apply(m.clone());
        }
        effect
    }

    /// The keyboard's Done, applying what it changes.
    fn type_text(ui: &mut Integrations, host: &mut Host, text: &str) -> Effect {
        let effect = ui.text_entered(text, host);
        if let Effect::Apply(m) = &effect {
            host.apply(m.clone());
        }
        effect
    }

    #[tokio::test]
    async fn adding_a_driver_and_putting_it_in_an_inputs_slots() {
        let (mut host, _fakes, mut ui) = setup();
        assert_eq!(labels(&ui.page(&host)), ["Global control", "Inputs", "Fake", "Add integration"]);
        tap(&mut ui, &mut host, "Add integration");
        assert!(matches!(tap(&mut ui, &mut host, "Fake"), Effect::Apply(_)));
        assert_eq!(labels(&ui.page(&host)), ["Host", "Remove Fake"], "straight into its setup");
        assert_eq!(ui.page(&host).rows[0].value, "Not set");
        let Effect::Text { title, text, .. } = tap(&mut ui, &mut host, "Host") else { panic!("asks for text") };
        assert_eq!((title.as_str(), text.as_str()), ("Host", ""));
        type_text(&mut ui, &mut host, "theater.local");
        assert_eq!(ui.page(&host).rows[0].value, "theater.local");

        ui.back();
        tap(&mut ui, &mut host, "Inputs");
        tap(&mut ui, &mut host, "KODI");
        tap(&mut ui, &mut host, "Screen");
        assert_eq!(labels(&ui.page(&host)), ["Nothing", "Fake"]);
        tap(&mut ui, &mut host, "Fake");
        assert_eq!(host.current("KODI").unwrap().ui.as_deref(), Some("fake-player"));
        assert_eq!(ui.page(&host).rows[5].value, "Fake", "back on the input, its screen slot filled");
        assert_eq!(ui.page(&host).focus, 5, "on the row it left from");
    }

    #[tokio::test]
    async fn inputs_are_renamed_hidden_moved_and_reset_with_a_mark() {
        let (mut host, _fakes, mut ui) = setup();
        tap(&mut ui, &mut host, "Inputs");
        assert_eq!(labels(&ui.page(&host)), ["TV", "KODI"]);
        tap(&mut ui, &mut host, "KODI");
        tap(&mut ui, &mut host, "Move up");
        tap(&mut ui, &mut host, "Show in the list");
        assert!(matches!(tap(&mut ui, &mut host, "Name"), Effect::Text { .. }));
        type_text(&mut ui, &mut host, "Kodi");
        assert_eq!(ui.page(&host).title, "Kodi");
        ui.back();
        let inputs = ui.page(&host);
        assert_eq!(labels(&inputs), ["Kodi", "TV"]);
        assert_eq!(inputs.focus, 0, "the focus followed it up");
        assert_eq!((inputs.rows[0].dot, inputs.rows[0].detail.as_str()), (Dot::Changed, "Hidden"));
        assert_eq!(inputs.rows[1].dot, Dot::Changed, "placed by the move too");
        tap(&mut ui, &mut host, "Kodi");
        tap(&mut ui, &mut host, "Reset");
        ui.back();
        assert_eq!(labels(&ui.page(&host)), ["TV", "KODI"]);
    }

    #[tokio::test]
    async fn global_control_is_picked_from_the_drivers_targets() {
        let (mut host, _fakes, mut ui) = setup();
        tap(&mut ui, &mut host, "Global control");
        let effect = tap(&mut ui, &mut host, "Volume");
        assert_eq!(effect, Effect::Load { driver: "fake-global".into(), kind: TargetKind::MediaPlayer });
        assert_eq!(labels(&ui.page(&host)), ["None", "Loading…"]);
        assert_eq!(ui.pick(1, &host), Effect::Stay, "nothing to pick while loading");
        ui.targets_loaded("fake-global", TargetKind::MediaPlayer, vec![Target { id: "media_player.amp".into(), name: "Amp".into() }]);
        tap(&mut ui, &mut host, "Amp");
        assert_eq!(host.modules().global.volume, Some(Binding { driver: "fake-global".into(), target: "media_player.amp".into() }));
        assert_eq!(ui.page(&host).rows[0].value, "Amp", "by the name its driver gave");
    }

    #[tokio::test]
    async fn pickers_show_what_is_bound_and_the_list_from_last_time_while_offline() {
        let (mut host, fakes, mut ui) = setup();
        let receiver = Target { id: "receiver".into(), name: "Receiver".into() };
        tap(&mut ui, &mut host, "Global control");
        tap(&mut ui, &mut host, "Inputs");
        let page = ui.page(&host);
        assert_eq!((labels(&page), page.rows[1].checked), (vec!["None", "receiver"], true), "bound, shown while loading");
        assert!(ui.targets_loaded("fake-global", TargetKind::MediaPlayer, vec![receiver.clone()]), "kept for next time");

        let mut ui = Integrations::with_known(ui.known().clone());
        ui.open();
        tap(&mut ui, &mut host, "Global control");
        assert_eq!(ui.page(&host).rows[1].value, "Receiver", "named as the driver last named it");
        fakes.get("fake-global").set_status(r3cui_sdk::api::Status::new(Health::Down));
        tap(&mut ui, &mut host, "Inputs");
        ui.loading("fake-global", TargetKind::MediaPlayer);
        assert!(!ui.targets_loaded("fake-global", TargetKind::MediaPlayer, Vec::new()), "nothing new to keep");
        let page = ui.page(&host);
        assert_eq!((page.title.as_str(), labels(&page)), ("Inputs · offline", vec!["None", "Receiver"]));
    }

    #[tokio::test]
    async fn an_empty_input_list_opens_where_it_is_fixed() {
        let fakes = Fakes::default();
        let mut m = Modules::default();
        m.add_driver("fake-global", Default::default());
        let mut host = Host::new(fakes.registry(), m.clone());
        let mut ui = Integrations::default();
        ui.open_for_inputs(&host);
        let page = ui.page(&host);
        assert_eq!((page.title.as_str(), page.focus), ("Global control", 1), "nothing is the source: on its Inputs row");

        m.bind(Role::Source, Some(Binding { driver: "fake-global".into(), target: "receiver".into() }));
        host.apply(m.clone());
        ui.open_for_inputs(&host);
        assert_eq!(ui.page(&host).title, "Fake", "the source has listed nothing: its own screen");
        assert_eq!(ui.back(), Effect::Stay);
        assert_eq!(ui.page(&host).focus, ROOT_DRIVERS, "back lands on its row");

        fakes.get("fake-global").set_sources(Sources { current: "TV".into(), list: vec!["TV".into()] });
        m.set_hidden("TV", true);
        host.apply(m);
        ui.open_for_inputs(&host);
        assert_eq!(ui.page(&host).title, "Inputs", "all hidden: the input list");
    }

    #[tokio::test]
    async fn power_is_a_third_row_that_follows_inputs_until_picked() {
        let (mut host, _fakes, mut ui) = setup();
        tap(&mut ui, &mut host, "Global control");
        let page = ui.page(&host);
        assert_eq!(labels(&page), ["Volume", "Inputs", "Power", "Voice"]);
        assert_eq!(page.rows[2].value, "receiver", "follows the inputs binding");
        tap(&mut ui, &mut host, "Power");
        ui.targets_loaded("fake-global", TargetKind::MediaPlayer, vec![Target { id: "media_player.tv".into(), name: "TV".into() }]);
        tap(&mut ui, &mut host, "TV");
        assert_eq!(host.modules().global.power, Some(Binding { driver: "fake-global".into(), target: "media_player.tv".into() }));
        assert_eq!(host.modules().global.source.as_ref().unwrap().target, "receiver", "inputs untouched");
        assert_eq!(ui.page(&host).rows[2].value, "TV", "by the name its driver gave");
    }

    #[tokio::test]
    async fn removing_asks_first_and_back_leaves_from_the_top() {
        let (mut host, _fakes, mut ui) = setup();
        tap(&mut ui, &mut host, "Fake");
        tap(&mut ui, &mut host, "Remove Fake");
        tap(&mut ui, &mut host, "Cancel");
        assert_eq!(ui.page(&host).title, "Fake");
        tap(&mut ui, &mut host, "Remove Fake");
        tap(&mut ui, &mut host, "Remove Fake");
        assert!(host.modules().drivers.is_empty() && host.modules().global.source.is_none());
        assert_eq!(ui.page(&host).title, "Integrations");
        assert_eq!(ui.back(), Effect::Leave);
    }

    fn pairable() -> (Host, Integrations) {
        let mut m = Modules::default();
        m.add_driver(FAKE_PAIRABLE.id, Default::default());
        let host = Host::new(Fakes::default().registry(), m);
        let mut ui = Integrations::default();
        ui.open();
        (host, ui)
    }

    #[tokio::test]
    async fn credentials_are_not_rows_and_pairing_has_its_own() {
        let (mut host, mut ui) = pairable();
        tap(&mut ui, &mut host, "Pairable");
        assert_eq!(labels(&ui.page(&host)), ["Host", "Pair", "Remove Pairable"]);
    }

    #[tokio::test]
    async fn setting_the_host_starts_pairing_and_the_code_finishes_it() {
        let (mut host, mut ui) = pairable();
        tap(&mut ui, &mut host, "Pairable");
        assert!(matches!(tap(&mut ui, &mut host, "Host"), Effect::Text { .. }));
        let Effect::Pair { driver, apply: Some(m) } = ui.text_entered("10.0.0.5", &host) else { panic!("goes on to pairing") };
        assert_eq!(driver, FAKE_PAIRABLE.id);
        host.apply(m);
        assert_eq!(ui.page(&host).title, "Pair Pairable");
        assert_eq!(labels(&ui.page(&host)), ["Contacting Pairable…"]);

        let Effect::Code { len, .. } = ui.pair_started(Ok(()), &host) else { panic!("asks for the code") };
        assert_eq!(len, 6);
        assert_eq!(ui.text_entered("1A2B3C", &host), Effect::Finish { code: "1A2B3C".into() });
        assert_eq!(labels(&ui.page(&host)), ["Checking the code…"]);

        let Effect::Code { caption, .. } = ui.pair_checked(Err(PairError::WrongCode), &host) else { panic!("asks again") };
        assert_eq!(caption, "WRONG CODE, TRY AGAIN");
        assert_eq!(ui.text_entered("1A2B3D", &host), Effect::Finish { code: "1A2B3D".into() });

        let Effect::Apply(m) = ui.pair_checked(Ok(vec![(FAKE_PAIRABLE_CREDENTIAL, "secret".into())]), &host) else { panic!("stores the credentials") };
        host.apply(m);
        assert_eq!(host.modules().drivers[FAKE_PAIRABLE.id].text(FAKE_PAIRABLE_CREDENTIAL).as_deref(), Some("secret"));
        assert_eq!(ui.page(&host).title, "Pairable", "back on the driver's page");
    }

    #[tokio::test]
    async fn a_failed_pairing_can_be_tried_again_and_a_paired_driver_is_not_paired_on_host_edit() {
        let (mut host, mut ui) = pairable();
        tap(&mut ui, &mut host, "Pairable");
        let Effect::Pair { .. } = tap(&mut ui, &mut host, "Pair") else { panic!("starts pairing") };
        assert_eq!(ui.pair_started(Err(PairError::Failed("No answer".into())), &host), Effect::Stay);
        assert_eq!(ui.page(&host).rows[0].detail, "No answer");
        assert!(matches!(tap(&mut ui, &mut host, "Try again"), Effect::Pair { apply: None, .. }));
        ui.back();
        assert_eq!(ui.page(&host).title, "Pairable");

        let mut m = host.modules().clone();
        m.set_field(FAKE_PAIRABLE.id, FAKE_PAIRABLE.field(FAKE_PAIRABLE_CREDENTIAL).unwrap(), "secret");
        host.apply(m);
        tap(&mut ui, &mut host, "Host");
        assert!(matches!(ui.text_entered("10.0.0.6", &host), Effect::Apply(_)), "already paired: just saved");
    }

    #[tokio::test]
    async fn power_can_be_bound_to_a_driver_that_offers_it() {
        let (mut host, mut ui) = pairable();
        tap(&mut ui, &mut host, "Global control");
        assert_eq!(ui.page(&host).rows[2].value, "None");
        tap(&mut ui, &mut host, "Power");
        assert_eq!(labels(&ui.page(&host)), ["None", "Pairable"]);
        tap(&mut ui, &mut host, "Pairable");
        assert_eq!(host.modules().global.power, Some(Binding { driver: FAKE_PAIRABLE.id.into(), target: FAKE_PAIRABLE.id.into() }));
        assert_eq!(ui.page(&host).rows[2].value, "Pairable");
        tap(&mut ui, &mut host, "Power");
        assert!(ui.page(&host).rows[1].checked);
        tap(&mut ui, &mut host, "None");
        assert_eq!(host.modules().global.power, None);
    }
}
