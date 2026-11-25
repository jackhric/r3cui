//! The on-screen QWERTY keyboard, driven by the d-pad (and touch). Pure state and geometry; the
//! window only draws the cells this produces.

const LETTERS: [&str; 3] = ["qwertyuiop", "asdfghjkl", "zxcvbnm"];
const SYMBOLS: [&str; 3] = ["1234567890", "@#$%&*-_/", "!?.,:'+="];
const KEY_W: f32 = 44.0;
const KEY_GAP: f32 = 3.0;
const KEY_H: f32 = 56.0;
const ROW_TOP: [f32; 3] = [330.0, 394.0, 458.0];
const ACTION_TOP: f32 = 548.0;
const ACTION_H: f32 = 64.0;
const ACTIONS: [(Kind, f32); 5] =
    [(Kind::Shift, 72.0), (Kind::Mode, 72.0), (Kind::Space, 132.0), (Kind::Delete, 72.0), (Kind::Join, 92.0)];
const ACTION_ROW: usize = 3;
/// WPA2 passphrases are 8 to 63 characters.
pub const MIN_LEN: usize = 8;
const MAX_LEN: usize = 63;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Char,
    Shift,
    Mode,
    Space,
    Delete,
    Join,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Char,
    Action,
    /// Shift while it is armed.
    ActionOn,
    JoinDim,
    JoinReady,
}

pub struct Cell {
    pub label: String,
    pub kind: Kind,
    pub style: Style,
    pub focused: bool,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Stay,
    /// BACK on an empty field.
    Leave,
    /// The action key: Join for a password, Done for other text.
    Done(String),
}

struct Key {
    kind: Kind,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

pub struct Keyboard {
    rows: Vec<Vec<Key>>,
    text: String,
    shift: bool,
    symbols: bool,
    focus: (usize, usize),
    /// The least text the action key accepts, and its label.
    min_len: usize,
    action: &'static str,
    /// A password is typed from nothing, so BACK rubs out; other text starts filled in, so BACK cancels.
    password: bool,
}

impl Default for Keyboard {
    fn default() -> Self {
        Self::new()
    }
}

impl Keyboard {
    pub fn new() -> Self {
        let mut rows: Vec<Vec<Key>> = LETTERS
            .iter()
            .zip(ROW_TOP)
            .map(|(letters, y)| {
                let n = letters.len() as f32;
                let x0 = (480.0 - (n * KEY_W + (n - 1.0) * KEY_GAP)) / 2.0;
                (0..letters.len())
                    .map(|i| Key { kind: Kind::Char, x: x0 + i as f32 * (KEY_W + KEY_GAP), y, w: KEY_W, h: KEY_H })
                    .collect()
            })
            .collect();
        let mut x = 12.0;
        rows.push(
            ACTIONS
                .iter()
                .map(|&(kind, w)| {
                    let key = Key { kind, x, y: ACTION_TOP, w, h: ACTION_H };
                    x += w + 4.0;
                    key
                })
                .collect(),
        );
        Self { rows, text: String::new(), shift: false, symbols: false, focus: (1, 4), min_len: MIN_LEN, action: "Join", password: true }
    }

    /// Ready for a Wi-Fi password.
    pub fn reset(&mut self) {
        self.start(MIN_LEN, "Join", "");
        self.password = true;
    }

    /// Ready for other text, starting from `text`; any length is accepted, empty included.
    pub fn start(&mut self, min_len: usize, action: &'static str, text: &str) {
        self.text = text.chars().take(MAX_LEN).collect();
        self.shift = false;
        self.symbols = false;
        self.focus = (1, 4);
        self.min_len = min_len;
        self.action = action;
        self.password = false;
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn ready(&self) -> bool {
        self.text.chars().count() >= self.min_len
    }

    fn char_at(&self, row: usize, col: usize) -> char {
        let set = if self.symbols { SYMBOLS } else { LETTERS };
        let c = set[row].chars().nth(col).unwrap_or(' ');
        if self.shift && !self.symbols {
            c.to_ascii_uppercase()
        } else {
            c
        }
    }

    pub fn cells(&self) -> Vec<Cell> {
        let mut out = Vec::new();
        for (r, row) in self.rows.iter().enumerate() {
            for (c, key) in row.iter().enumerate() {
                let (label, style) = match key.kind {
                    Kind::Char => (self.char_at(r, c).to_string(), Style::Char),
                    Kind::Shift => (String::new(), if self.shift && !self.symbols { Style::ActionOn } else { Style::Action }),
                    Kind::Mode => (if self.symbols { "abc" } else { "123" }.to_string(), Style::Action),
                    Kind::Space => ("space".to_string(), Style::Action),
                    Kind::Delete => (String::new(), Style::Action),
                    Kind::Join => (self.action.to_string(), if self.ready() { Style::JoinReady } else { Style::JoinDim }),
                };
                out.push(Cell { label, kind: key.kind, style, focused: self.focus == (r, c), x: key.x, y: key.y, w: key.w, h: key.h });
            }
        }
        out
    }

    /// `name` is one of up, down, left, right, ok, back.
    pub fn key(&mut self, name: &str) -> Outcome {
        let (r, c) = self.focus;
        match name {
            "left" => self.focus.1 = (c + self.rows[r].len() - 1) % self.rows[r].len(),
            "right" => self.focus.1 = (c + 1) % self.rows[r].len(),
            "up" => self.move_row((r + self.rows.len() - 1) % self.rows.len()),
            "down" => self.move_row((r + 1) % self.rows.len()),
            "ok" => return self.press(r, c),
            "back" if !self.password => return Outcome::Leave,
            "back" if self.text.pop().is_none() => return Outcome::Leave,
            _ => {}
        }
        Outcome::Stay
    }

    /// Touch: `index` counts cells in the order `cells()` returns them.
    pub fn tap(&mut self, index: usize) -> Outcome {
        let mut i = index;
        for r in 0..self.rows.len() {
            if i < self.rows[r].len() {
                self.focus = (r, i);
                return self.press(r, i);
            }
            i -= self.rows[r].len();
        }
        Outcome::Stay
    }

    /// The rows are staggered, so vertical moves go to the key whose centre is nearest.
    fn move_row(&mut self, to: usize) {
        let from = &self.rows[self.focus.0][self.focus.1];
        let cx = from.x + from.w / 2.0;
        let nearest = self.rows[to]
            .iter()
            .enumerate()
            .min_by(|a, b| {
                let d = |k: &Key| (k.x + k.w / 2.0 - cx).abs();
                d(a.1).total_cmp(&d(b.1))
            })
            .map_or(0, |(i, _)| i);
        self.focus = (to, nearest);
    }

    fn press(&mut self, r: usize, c: usize) -> Outcome {
        match self.rows[r][c].kind {
            Kind::Char => {
                let ch = self.char_at(r, c);
                self.push(ch);
                self.shift = false;
            }
            Kind::Space => self.push(' '),
            Kind::Shift => self.shift = !self.shift && !self.symbols,
            Kind::Mode => {
                self.symbols = !self.symbols;
                self.focus = (ACTION_ROW, 1);
            }
            Kind::Delete => {
                self.text.pop();
            }
            Kind::Join if self.ready() => return Outcome::Done(self.text.clone()),
            Kind::Join => {}
        }
        Outcome::Stay
    }

    fn push(&mut self, c: char) {
        if self.text.chars().count() < MAX_LEN {
            self.text.push(c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(kb: &mut Keyboard, keys: &[&str]) -> Outcome {
        keys.iter().map(|k| kb.key(k)).last().unwrap_or(Outcome::Stay)
    }

    #[test]
    fn starts_on_g_and_types() {
        let mut kb = Keyboard::new();
        run(&mut kb, &["ok", "right", "ok"]);
        assert_eq!(kb.text(), "gh");
    }

    #[test]
    fn rows_wrap_sideways() {
        let mut kb = Keyboard::new();
        run(&mut kb, &["up", "left", "left", "left", "left", "left", "ok"]);
        assert_eq!(kb.text(), "p", "left of q wraps to p");
    }

    #[test]
    fn vertical_moves_pick_the_nearest_key() {
        let mut kb = Keyboard::new();
        // g sits between t and y on the row above, and above v/b below.
        run(&mut kb, &["down", "ok"]);
        assert_eq!(kb.text(), "v");
        let mut kb = Keyboard::new();
        run(&mut kb, &["up", "up", "ok"]);
        assert_eq!(kb.text(), " ", "above the top row is the action row; g lines up with space");
    }

    #[test]
    fn shift_applies_to_one_letter() {
        let mut kb = Keyboard::new();
        kb.tap(26);
        run(&mut kb, &["up", "ok", "ok"]);
        assert_eq!(kb.text(), "Zz");
    }

    #[test]
    fn join_needs_eight_characters() {
        let mut kb = Keyboard::new();
        for _ in 0..7 {
            kb.key("ok");
        }
        assert_eq!(kb.tap(30), Outcome::Stay);
        kb.tap(14);
        assert_eq!(kb.tap(30), Outcome::Done("gggggggg".to_string()));
    }

    #[test]
    fn back_deletes_then_leaves() {
        let mut kb = Keyboard::new();
        kb.key("ok");
        assert_eq!(kb.key("back"), Outcome::Stay);
        assert_eq!(kb.key("back"), Outcome::Leave);
    }

    #[test]
    fn symbols_mode() {
        let mut kb = Keyboard::new();
        kb.tap(27);
        kb.tap(0);
        assert_eq!(kb.text(), "1");
    }

    #[test]
    fn other_text_starts_from_what_is_there_and_may_be_empty() {
        let mut kb = Keyboard::new();
        kb.start(0, "Done", "kodi");
        assert_eq!(kb.text(), "kodi");
        assert!(kb.ready());
        let done = kb.cells().iter().position(|c| c.kind == Kind::Join).unwrap();
        assert_eq!(kb.cells()[done].label, "Done");
        assert_eq!(kb.key("back"), Outcome::Leave, "cancels rather than rubbing out");
        kb.start(0, "Done", "");
        assert_eq!(kb.tap(done), Outcome::Done(String::new()));
    }
}
