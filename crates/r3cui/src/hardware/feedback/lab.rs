use super::cue::{Cue, Strength};
use super::worker::{play, play_at, raw, stop};

/// State behind the Feedback lab: a main screen and two sub-screens.
pub(crate) struct Lab {
    screen: LabScreen,
    strength: Option<Strength>,
    buzzing: bool,
    raw: u8,
}

impl Default for Lab {
    fn default() -> Self {
        Self { screen: LabScreen::Main, strength: None, buzzing: false, raw: 1 }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum LabScreen {
    #[default]
    Main,
    Patterns,
    Raw,
}

/// One lab row: label, second line, value on the right, and whether it opens a screen.
pub(crate) struct LabRow {
    pub(crate) label: String,
    pub(crate) detail: String,
    pub(crate) value: String,
    pub(crate) opens: bool,
}

fn row(label: &str, detail: &str, value: &str, opens: bool) -> LabRow {
    LabRow { label: label.into(), detail: detail.into(), value: value.into(), opens }
}

const ROM_EFFECTS: i32 = 123;

/// The cues the remote itself uses, in the order the Patterns screen lists them.
const UI_CUES: [Cue; 11] =
    [Cue::Tap, Cue::Volume, Cue::Tick, Cue::Select, Cue::Back, Cue::Limit, Cue::Success, Cue::Error, Cue::HoldRamp, Cue::HoldComplete, Cue::Rumble];
/// Named effects with no place in the UI yet, to feel on the Raw effects screen.
const EXTRA_CUES: [Cue; 6] = [Cue::Alert, Cue::LongBuzz, Cue::RampUp, Cue::RampDown, Cue::Triple, Cue::Double];

impl Cue {
    /// Where the remote uses this cue.
    fn purpose(self) -> &'static str {
        match self {
            Self::Tap => "Arriving on a new page",
            Self::Volume => "Every volume change",
            Self::Tick => "Not used now",
            Self::Select => "OK, choosing a row, slider double tap",
            Self::Back => "Not used now",
            Self::Limit => "Slider at silence or the volume limit",
            Self::Success => "Power hold completed",
            Self::Error => "Something failed (not used yet)",
            Self::HoldRamp => "Holding Home",
            Self::HoldComplete => "Home hold completed",
            Self::Rumble => "Holding power",
            _ => "",
        }
    }
}

impl Lab {
    pub(crate) fn title(&self) -> &'static str {
        match self.screen {
            LabScreen::Main => "Feedback lab",
            LabScreen::Patterns => "Patterns",
            LabScreen::Raw => "Raw effects",
        }
    }

    pub(crate) fn rows(&self) -> Vec<LabRow> {
        let strength = self.strength.map_or("As designed", Strength::name);
        match self.screen {
            LabScreen::Main => vec![
                row("Patterns", "Play each cue the remote uses", "", true),
                row("Strength", "Only on this page", strength, false),
                row("Constant buzz", "Select to start or stop", if self.buzzing { "On" } else { "Off" }, false),
                row("Raw effects", "Every motor effect, by number", "", true),
            ],
            LabScreen::Patterns => UI_CUES
                .iter()
                .map(|c| {
                    let shown = self.strength.unwrap_or(c.spec().strength);
                    row(c.name(), c.purpose(), shown.name(), false)
                })
                .collect(),
            LabScreen::Raw => {
                let mut rows = vec![
                    row(&format!("Play effect {}", self.raw), "Select to feel it", "", false),
                    row("Next", "", &format!("{}", self.step(1)), false),
                    row("Previous", "", &format!("{}", self.step(-1)), false),
                    row("Skip ahead 10", "", &format!("{}", self.step(10)), false),
                ];
                rows.extend(EXTRA_CUES.iter().map(|c| row(c.name(), "", "", false)));
                rows
            }
        }
    }

    fn step(&self, by: i32) -> u8 {
        ((self.raw as i32 - 1 + by).rem_euclid(ROM_EFFECTS) + 1) as u8
    }

    /// Back from a sub-screen to the main one, returning the row to focus there (the row that
    /// opened it); `None` when already on the main screen.
    pub(crate) fn back(&mut self) -> Option<usize> {
        let focus = match self.screen {
            LabScreen::Main => return None,
            LabScreen::Patterns => 0,
            LabScreen::Raw => 3,
        };
        self.screen = LabScreen::Main;
        Some(focus)
    }

    /// Returns true when the screen changed.
    pub(crate) fn pick(&mut self, row: usize) -> bool {
        match self.screen {
            LabScreen::Main => match row {
                0 => {
                    self.screen = LabScreen::Patterns;
                    return true;
                }
                1 => {
                    self.strength = match self.strength {
                        None => Some(Strength::Soft),
                        Some(Strength::Soft) => Some(Strength::Medium),
                        Some(Strength::Medium) => Some(Strength::Strong),
                        Some(Strength::Strong) => None,
                    };
                    if let Some(s) = self.strength {
                        play_at(Cue::Select, s);
                    }
                }
                2 => {
                    self.buzzing = !self.buzzing;
                    if self.buzzing {
                        self.play(Cue::Rumble);
                    } else {
                        stop();
                    }
                }
                3 => {
                    self.screen = LabScreen::Raw;
                    return true;
                }
                _ => {}
            },
            LabScreen::Patterns => {
                if let Some(cue) = UI_CUES.get(row) {
                    self.play(*cue);
                }
            }
            LabScreen::Raw => match row {
                0 => raw(self.raw),
                1 => {
                    self.raw = self.step(1);
                    raw(self.raw);
                }
                2 => {
                    self.raw = self.step(-1);
                    raw(self.raw);
                }
                3 => {
                    self.raw = self.step(10);
                    raw(self.raw);
                }
                n => {
                    if let Some(cue) = EXTRA_CUES.get(n - 4) {
                        self.play(*cue);
                    }
                }
            },
        }
        false
    }

    fn play(&self, cue: Cue) {
        match self.strength {
            Some(s) => play_at(cue, s),
            None => play(cue),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lab_screens_open_and_go_back() {
        let mut lab = Lab::default();
        assert_eq!(lab.rows().len(), 4);
        assert!(!lab.pick(1));
        assert_eq!(lab.rows()[1].value, "Soft");
        assert!(lab.pick(0));
        assert_eq!(lab.title(), "Patterns");
        assert_eq!(lab.rows().len(), UI_CUES.len());
        assert_eq!(lab.rows()[0].value, "Soft");
        assert_eq!(lab.back(), Some(0));
        assert!(lab.pick(3));
        assert_eq!(lab.rows()[1].value, "2");
        lab.pick(2);
        assert_eq!(lab.rows()[0].label, "Play effect 123");
        assert_eq!(lab.back(), Some(3));
        assert_eq!(lab.back(), None);
    }
}
