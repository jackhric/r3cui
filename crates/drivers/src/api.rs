//! The contract between r3cui's frontend and its drivers. A driver offers ports; each port is state out on a
//! `watch` and intents in on a channel. Nothing here names a driver.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};

pub struct Port<V, I> {
    pub view: watch::Receiver<V>,
    pub tx: mpsc::UnboundedSender<I>,
}

impl<V, I> Clone for Port<V, I> {
    fn clone(&self) -> Self {
        Self { view: self.view.clone(), tx: self.tx.clone() }
    }
}

impl<V: Clone, I> Port<V, I> {
    pub fn get(&self) -> V {
        self.view.borrow().clone()
    }

    pub fn send(&self, intent: I) {
        let _ = self.tx.send(intent);
    }
}

/// A new port: the driver keeps the sender and the receiver, and hands out the port.
pub fn port<V, I>(initial: V) -> (watch::Sender<V>, mpsc::UnboundedReceiver<I>, Port<V, I>) {
    let (view_tx, view) = watch::channel(initial);
    let (tx, rx) = mpsc::unbounded_channel();
    (view_tx, rx, Port { view, tx })
}

/// On a 0–100 scale. `max` is the highest level allowed (a receiver's own volume limit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    pub level: i32,
    pub max: i32,
    pub muted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VolumeIntent {
    Up,
    Down,
    Set(i32),
    Mute(bool),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Sources {
    pub current: String,
    pub list: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceIntent {
    Select(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Power {
    pub on: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerIntent {
    Set(bool),
}

pub type VolumePort = Port<Option<Volume>, VolumeIntent>;
pub type SourcePort = Port<Option<Sources>, SourceIntent>;
pub type PowerPort = Port<Option<Power>, PowerIntent>;
pub type UiPort = Port<Stage, UiIntent>;

/// The buttons that go to the current input's remote slot. Volume, mute, power, Home and the input key are the
/// remote's own and never reach a driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteKey {
    Up,
    Down,
    Left,
    Right,
    Select,
    Back,
    Menu,
    ChannelUp,
    ChannelDown,
    Previous,
    PlayPause,
    Next,
    Stop,
    Record,
}

/// What an input's UI slot puts on the stage. Neither part: a blank stage.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Stage {
    pub media: Option<Media>,
    pub library: Option<Library>,
    pub app: Option<App>,
}

/// The program in front on a box that runs apps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Media {
    pub title: String,
    /// "Season 1 · Episode 4" and the like; empty for a film.
    pub subtitle: String,
    pub paused: bool,
    pub position: Duration,
    /// When `position` was true; it runs on from there unless paused.
    pub measured_at: Instant,
    pub total: Duration,
    /// The audio format as the codec badges name it ("TrueHD Atmos 7.1").
    pub format: String,
    pub poster: String,
    pub backdrop: String,
    pub audio: Vec<Track>,
    pub audio_current: Option<usize>,
    pub subtitles: Vec<Track>,
    pub subtitle_current: Option<usize>,
}

impl Media {
    pub fn position_at(&self, now: Instant) -> Duration {
        if self.paused {
            return self.position;
        }
        (self.position + now.saturating_duration_since(self.measured_at)).min(self.total.max(self.position))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub label: String,
    pub detail: String,
}

/// A browsable list on the stage, with the highlighted entry lifted out.
#[derive(Debug, Clone, PartialEq)]
pub struct Library {
    pub list: Arc<LibraryList>,
    pub focus: usize,
}

pub const MOVIES_HEADING: &str = "MOVIES";
pub const TV_SHOWS_HEADING: &str = "TV SHOWS";

#[derive(Debug, PartialEq)]
pub struct LibraryList {
    /// Changes whenever the list does, so the frontend reloads only then.
    pub id: u64,
    pub heading: String,
    pub count: String,
    pub items: Vec<LibraryItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LibraryItem {
    pub title: String,
    pub meta: String,
    pub rating: String,
    /// The certificate, named as the badges are ("PG-13").
    pub tail: String,
    pub genre: String,
    pub plot: String,
    pub detail: String,
    pub progress: f32,
    pub poster: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiIntent {
    /// Whether the slot is on screen and the remote awake: a driver only follows its program while it is.
    Shown(bool),
    PlayPause,
    Seek(i32),
    Audio(usize),
    Subtitle(Option<usize>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyMode {
    #[default]
    Follow,
    Custom,
    Off,
}

/// The key backlight as another program asks for it: follow the remote, a colour, or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyLight {
    pub mode: KeyMode,
    /// Already scaled by brightness; `None` while the light is off.
    pub colour: Option<(u8, u8, u8)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Connecting,
    Up,
    Down,
    NotSetUp,
    /// Reachable, but the device has to be paired with first.
    NotPaired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub health: Health,
    /// What to show instead of the health's own word, such as "Token rejected".
    pub detail: String,
}

impl Status {
    pub fn new(health: Health) -> Self {
        Self { health, detail: String::new() }
    }

    pub fn with(health: Health, detail: impl Into<String>) -> Self {
        Self { health, detail: detail.into() }
    }

    pub fn word(&self) -> &str {
        if !self.detail.is_empty() {
            return &self.detail;
        }
        match self.health {
            Health::Connecting => "Connecting",
            Health::Up => "Connected",
            Health::Down => "Unreachable",
            Health::NotSetUp => "Not set up",
            Health::NotPaired => "Needs pairing",
        }
    }
}

/// What a target field picks from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    MediaPlayer,
    Light,
    Select,
    Sensor,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Target {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Url,
    Port,
    /// Never shown back once set.
    Secret,
    /// Written by pairing and by nothing else: not listed, never typed.
    Credential,
    Target(TargetKind),
}

#[derive(Debug, Clone, Copy)]
pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: FieldKind,
    pub required: bool,
    pub default: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Provides {
    pub global: bool,
    pub remote: bool,
    pub ui: bool,
    /// Can stand as global power without being global control.
    pub power: bool,
}

/// A driver that has to be paired with its device: the device shows a code, the user types it in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodePairing {
    pub code_len: usize,
}

/// A driver type: what it is called, what setting it up asks for, and which jobs it can do.
#[derive(Debug)]
pub struct Spec {
    pub id: &'static str,
    pub name: &'static str,
    /// Its mark: 24×24 SVG path data, filled in one colour.
    pub icon: &'static str,
    pub fields: &'static [Field],
    /// The field that says where the device is, named when it cannot be reached.
    pub address: Option<&'static str>,
    pub provides: Provides,
    pub pairing: Option<CodePairing>,
}

impl Spec {
    pub fn field(&self, key: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.key == key)
    }

    /// The fields the user can see and change: all but the credentials.
    pub fn visible_fields(&self) -> impl Iterator<Item = &Field> {
        self.fields.iter().filter(|f| f.kind != FieldKind::Credential)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairError {
    /// Type it again.
    WrongCode,
    /// Start over.
    Failed(String),
}

/// What a finished pairing leaves in the driver's setup, by field key.
pub type Paired = Vec<(&'static str, String)>;

/// A pairing under way: the device is showing its code.
pub trait Pairer: Send {
    fn finish<'a>(&'a mut self, code: &'a str) -> Pin<Box<dyn Future<Output = Result<Paired, PairError>> + Send + 'a>>;
}

pub type PairStart = Pin<Box<dyn Future<Output = Result<Box<dyn Pairer>, PairError>> + Send>>;

pub type Targets<'a> = Pin<Box<dyn Future<Output = Vec<Target>> + Send + 'a>>;

/// A running driver.
pub trait Driver: Send + Sync {
    fn spec(&self) -> &'static Spec;
    fn status(&self) -> watch::Receiver<Status>;
    fn volume(&self, _target: &str) -> Option<VolumePort> {
        None
    }
    fn source(&self, _target: &str) -> Option<SourcePort> {
        None
    }
    fn power(&self, _target: &str) -> Option<PowerPort> {
        None
    }
    fn remote(&self) -> Option<mpsc::UnboundedSender<RemoteKey>> {
        None
    }
    fn ui(&self) -> Option<UiPort> {
        None
    }
    /// Starts pairing, for a driver whose spec has `pairing`.
    fn pair(&self) -> Option<PairStart> {
        None
    }
    fn key_light(&self) -> Option<watch::Receiver<Option<KeyLight>>> {
        None
    }
    /// The mark shown for `input` while this driver fills its slot; by default the driver's own, empty for none.
    fn mark(&self, _input: &str) -> &'static str {
        self.spec().icon
    }
    /// What a target field of this driver can point at, for the pickers.
    fn targets(&self, _kind: TargetKind) -> Targets<'_> {
        Box::pin(async { Vec::new() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_runs_on_unless_paused() {
        let t = Instant::now();
        let mut m = Media {
            title: "x".into(),
            subtitle: String::new(),
            paused: false,
            position: Duration::from_secs(10),
            measured_at: t,
            total: Duration::from_secs(12),
            format: String::new(),
            poster: String::new(),
            backdrop: String::new(),
            audio: vec![],
            audio_current: None,
            subtitles: vec![],
            subtitle_current: None,
        };
        assert_eq!(m.position_at(t + Duration::from_secs(1)), Duration::from_secs(11));
        assert_eq!(m.position_at(t + Duration::from_secs(9)), Duration::from_secs(12), "stops at the end");
        m.paused = true;
        assert_eq!(m.position_at(t + Duration::from_secs(1)), Duration::from_secs(10));
    }
}
