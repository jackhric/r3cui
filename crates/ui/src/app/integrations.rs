//! Settings → Integrations: the drivers set up, global control, and the inputs with their slots. Every screen is a
//! list drawn from the host and the driver specs, so a new driver needs no new screen. This is the logic alone: it
//! says what to show and what to do, and the session does it.

use drivers::api::{FieldKind, Health, Spec, Target, TargetKind};
use drivers::{Binding, Host, Input, Modules, Role, Slot};

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
}

/// A status dot: working, broken, unknown, or changed by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Dot {
    #[default]
    None,
    Up,
    Down,
    Unknown,
    Changed,
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
}

#[derive(Debug, PartialEq)]
pub(crate) enum Effect {
    Stay,
    Apply(Modules),
    /// Fetch what a target field can point at from `driver`; `targets_loaded` brings them.
    Load { driver: String, kind: TargetKind },
    Text { title: String, caption: String, text: String },
    Leave,
}

#[derive(Debug, Clone, PartialEq)]
enum Editing {
    Field { driver: String, key: &'static str },
    Name(String),
}

#[derive(Default)]
pub(crate) struct Integrations {
    /// The screens open, each with its focused row.
    stack: Vec<(Screen, usize)>,
    /// What the open picker offers; `None` while loading.
    targets: Option<Vec<Target>>,
    editing: Option<Editing>,
}

fn row(label: impl Into<String>) -> Row {
    Row { label: label.into(), ..Default::default() }
}

fn spec_of(host: &Host, driver: &str) -> Option<&'static Spec> {
    host.registry().spec(&host.modules().drivers.get(driver)?.kind)
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

fn input_of(host: &Host, id: &str) -> Option<Input> {
    host.inputs().into_iter().find(|i| i.id == id)
}

impl Integrations {
    /// Opens on the first screen.
    pub(crate) fn open(&mut self) {
        (self.stack, self.targets, self.editing) = (vec![(Screen::Root, 0)], None, None);
    }

    fn top(&self) -> &Screen {
        self.stack.last().map_or(&Screen::Root, |(s, _)| s)
    }

    fn push(&mut self, screen: Screen) {
        self.targets = None;
        self.stack.push((screen, 0));
    }

    pub(crate) fn targets_loaded(&mut self, targets: Vec<Target>) {
        self.targets = Some(targets);
    }

    pub(crate) fn page(&self, host: &Host) -> Page {
        let m = host.modules();
        let focus = self.stack.last().map_or(0, |(_, f)| *f);
        let (title, rows) = match self.top() {
            Screen::Root => {
                let mut rows = vec![
                    Row { value: m.global.source.as_ref().or(m.global.volume.as_ref()).map_or("None".into(), |b| short(&b.target)), chevron: true, ..row("Global control") },
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
                let target = |b: &Option<Binding>| b.as_ref().map_or("None".to_string(), |b| short(&b.target));
                let rows = vec![
                    Row { value: target(&m.global.volume), chevron: true, detail: "Volume and mute".into(), ..row("Volume") },
                    Row { value: target(&m.global.source), chevron: true, detail: "Inputs and power".into(), ..row("Source") },
                ];
                ("Global control".to_string(), rows)
            }
            Screen::PickRole(role) => {
                let bound = match role {
                    Role::Volume => &m.global.volume,
                    Role::Source => &m.global.source,
                };
                let title = if *role == Role::Volume { "Volume" } else { "Source" };
                if global_driver(host).is_none() {
                    (title.to_string(), vec![row("Add Home Assistant first")])
                } else {
                    (title.to_string(), self.target_rows(bound.as_ref().map(|b| b.target.as_str())))
                }
            }
            Screen::Driver(id) => {
                let entry = &m.drivers[id];
                let mut rows: Vec<Row> = spec_of(host, id)
                    .map(|s| s.fields)
                    .unwrap_or_default()
                    .iter()
                    .map(|f| {
                        let value = match (f.kind, entry.text(f.key)) {
                            (FieldKind::Secret, Some(_)) => "Set".to_string(),
                            (FieldKind::Secret, None) | (_, None) if f.required => "Not set".to_string(),
                            (_, None) => "None".to_string(),
                            (_, Some(v)) => v,
                        };
                        Row { value, chevron: true, ..row(f.label) }
                    })
                    .collect();
                rows.push(row(format!("Remove {}", name_of(host, id))));
                (name_of(host, id), rows)
            }
            Screen::PickField { driver, key } => {
                let label = spec_of(host, driver).and_then(|s| s.fields.iter().find(|f| f.key == *key)).map_or(*key, |f| f.label);
                (label.to_string(), self.target_rows(m.drivers.get(driver).and_then(|d| d.text(key)).as_deref()))
            }
            Screen::Remove(id) => (format!("Remove {}?", name_of(host, id)), vec![row(format!("Remove {}", name_of(host, id))), row("Cancel")]),
            Screen::Add => {
                let rows = self
                    .addable(host)
                    .map(|s| {
                        let jobs: Vec<&str> =
                            [(s.provides.global, "volume and inputs"), (s.provides.remote, "buttons"), (s.provides.ui, "screen")].into_iter().filter(|(y, _)| *y).map(|(_, j)| j).collect();
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
        Page { title, rows, focus }
    }

    /// "None", then what the picker offers, the current one ticked.
    fn target_rows(&self, current: Option<&str>) -> Vec<Row> {
        let mut rows = vec![Row { checked: current.is_none(), ..row("None") }];
        match &self.targets {
            None => rows.push(row("Loading…")),
            Some(t) if t.is_empty() => rows.push(row("Nothing found")),
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
                    0 => self.push(Screen::Global),
                    1 => self.push(Screen::Inputs),
                    n if n - 2 < drivers.len() => self.push(Screen::Driver(drivers[n - 2].clone())),
                    _ if self.addable(host).next().is_some() => self.push(Screen::Add),
                    _ => {}
                }
                Effect::Stay
            }
            Screen::Global => {
                let role = if at == 0 { Role::Volume } else { Role::Source };
                self.push(Screen::PickRole(role));
                match global_driver(host) {
                    Some(driver) => Effect::Load { driver, kind: TargetKind::MediaPlayer },
                    None => Effect::Stay,
                }
            }
            Screen::PickRole(role) => {
                let (Some(driver), Some(target)) = (global_driver(host), self.picked_target(at)) else { return Effect::Stay };
                m.bind(role, target.map(|target| Binding { driver, target }));
                self.pop();
                Effect::Apply(m)
            }
            Screen::Driver(id) => {
                let fields = spec_of(host, &id).map(|s| s.fields).unwrap_or_default();
                match fields.get(at) {
                    Some(f) => match f.kind {
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
                m.set_field(&driver, key, target.as_deref().unwrap_or(""));
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

    /// The keyboard's Done.
    pub(crate) fn text_entered(&mut self, text: &str, host: &Host) -> Option<Modules> {
        let mut m = host.modules().clone();
        match self.editing.take()? {
            Editing::Field { driver, key } => m.set_field(&driver, key, text),
            Editing::Name(input) => m.rename(&input, text),
        }
        Some(m)
    }

    fn pop(&mut self) {
        self.targets = None;
        self.stack.pop();
    }

    /// One screen back; `Leave` from the first.
    pub(crate) fn back(&mut self) -> Effect {
        self.pop();
        if self.stack.is_empty() { Effect::Leave } else { Effect::Stay }
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
    use drivers::api::Sources;
    use drivers::testing::Fakes;

    fn setup() -> (Host, Fakes, Integrations) {
        let fakes = Fakes::default();
        let mut m = Modules::default();
        m.add_driver("fake-global", Default::default());
        m.bind(Role::Source, Some(Binding { driver: "fake-global".into(), target: "receiver".into() }));
        let host = Host::new(fakes.registry(), m);
        fakes.get("fake-global").set_sources(Sources { on: true, current: "TV".into(), list: vec!["TV".into(), "KODI".into()] });
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
        host.apply(ui.text_entered("theater.local", &host).unwrap());
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
        host.apply(ui.text_entered("Kodi", &host).unwrap());
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
        ui.targets_loaded(vec![Target { id: "media_player.amp".into(), name: "Amp".into() }]);
        tap(&mut ui, &mut host, "Amp");
        assert_eq!(host.modules().global.volume, Some(Binding { driver: "fake-global".into(), target: "media_player.amp".into() }));
        assert_eq!(ui.page(&host).rows[0].value, "amp");
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
}
