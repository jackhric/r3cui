//! What the System pages know: whether r3cui and the remote's firmware have updates, how an install is going, and the
//! web configurator's PIN. Pure: the session feeds it what the UI asked for (`Action`) and what the work it started
//! reported (`Event`), and runs the `Command`s it answers with.

use crate::app::Later;
use crate::remote::link::Link;
use crate::remote::selfupdate::{Release, REPO_PATH};
use crate::remote::software::Progress;
use crate::view::format::{ago, release_date, size_label};
use crate::view::screen::Screen;
use crate::view::ui;
use crate::{AppWindow, InstallPhase, PageId, UninstallState, UpdateState, VoiceSetup};
use crate::coreapi::{DownloadState, SoftwareUpdate, SystemUpdateInfo, UpdateEventType, UpdateRefusal, UpdateRunState, UpdateStart};
use semver::Version;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

const R3_CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);
/// The core looks for firmware updates itself; asking for its answer again is cheap, so the page asks when it opens
/// if the last answer is older than this.
const FIRMWARE_ASK_EVERY: Duration = Duration::from_secs(300);
/// What the core wants of the battery before it installs firmware.
pub(crate) const BATTERY_NEEDED: i32 = 50;
/// The language the release notes are shown in.
const NOTES_LANGUAGE: &str = "en";
/// The configurator's address on the remote.
const CONFIGURATOR_PATH: &str = "/configurator";

pub(crate) enum Action {
    /// The System page opened.
    Opened,
    CheckR3,
    InstallR3,
    CheckFirmware,
    InstallFirmware,
    /// The web configurator page opened.
    OpenWeb,
    NewPin,
    WebOff,
    Uninstall,
    InstallVoice,
}

#[derive(Debug)]
pub(crate) enum Event {
    R3Checked(Result<Option<Release>, String>),
    R3Progress { phase: InstallPhase, percent: u8 },
    R3Failed(String),
    UninstallFailed(String),
    Firmware(Result<SystemUpdateInfo, String>),
    FirmwareUpdate(SoftwareUpdate),
    FirmwareStarted(Result<UpdateStart, UpdateRefusal>),
    WebRead(bool),
    WebSet(Result<(), String>),
    Address(String),
    VoiceRead(Result<bool, String>),
    VoiceInstalled(Result<(), String>),
}

/// What the session is to do, in the order given.
#[derive(Debug, PartialEq)]
pub(crate) enum Command {
    CheckR3,
    InstallR3 { release: Release, access: Access },
    CheckFirmware { force: bool },
    StartFirmware { id: String },
    ReadWeb,
    ReadAddress,
    /// Turn the web configurator on with this PIN, or off.
    SetWeb { pin: Option<String> },
    Uninstall { access: Access },
    ReadVoice,
    InstallVoice { access: Access },
    Go(PageId),
}

/// How an install or an uninstall gets past the core's REST login: with the PIN it needs the web configurator on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Access {
    pub(crate) pin: String,
    /// The configurator is off, or its PIN is not known: switch it on with `pin` first.
    pub(crate) enable: bool,
    /// It was off before, so switch it off again afterwards.
    pub(crate) restore_off: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Check {
    Never,
    Running,
    Done { at: Instant, ok: bool },
}

impl Check {
    fn due(self, now: Instant, every: Duration) -> bool {
        match self {
            Self::Never => true,
            Self::Running => false,
            Self::Done { at, ok } => !ok || now.duration_since(at) >= every,
        }
    }

    fn state(self, has_update: bool) -> UpdateState {
        match self {
            _ if has_update => UpdateState::Available,
            Self::Never => UpdateState::Unchecked,
            Self::Running => UpdateState::Checking,
            Self::Done { ok: true, .. } => UpdateState::Current,
            Self::Done { ok: false, .. } => UpdateState::Failed,
        }
    }

    /// The update tile's second line: the release's size when there is one, otherwise when it last looked.
    fn short(self, now: Instant, release: Option<&Release>) -> String {
        match (self, release.map(|r| r.size).filter(|&size| size > 0)) {
            (_, Some(size)) => size_label(size),
            (Self::Never, _) => "Not checked yet".into(),
            (Self::Running, _) => String::new(),
            (Self::Done { ok: false, .. }, _) => "Couldn’t check".into(),
            (Self::Done { at, .. }, _) => ago(now.duration_since(at)),
        }
    }

    fn line(self, now: Instant, has_update: bool, new: &str) -> String {
        match self {
            _ if has_update => format!("Update available · {new}"),
            Self::Never => "Not checked yet".into(),
            Self::Running => "Checking…".into(),
            Self::Done { ok: false, .. } => "Couldn’t check for updates".into(),
            Self::Done { at, .. } => format!("Up to date · checked {}", ago(now.duration_since(at))),
        }
    }
}

struct R3 {
    check: Check,
    release: Option<Release>,
    error: Option<String>,
    install: Option<(InstallPhase, u8)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    /// `update_system` sent, not yet answered.
    Requested,
    Downloading,
    /// Downloaded; `update_system` sent again to install it.
    Starting,
    Running,
}

struct FirmwareInstall {
    stage: Stage,
    phase: InstallPhase,
    percent: u8,
    step: Option<(u32, u32)>,
}

struct Offer {
    id: String,
    version: String,
    release_date: String,
    size: u64,
    notes: String,
    download: DownloadState,
}

struct Firmware {
    installed: String,
    check: Check,
    offer: Option<Offer>,
    error: Option<String>,
    install: Option<FirmwareInstall>,
}

#[derive(Default)]
struct Web {
    enabled: bool,
    pin: Option<String>,
    address: String,
    error: Option<String>,
}

pub(crate) struct Software {
    running: Version,
    r3: R3,
    firmware: Firmware,
    web: Web,
    uninstall: (UninstallState, String),
    /// the r3cui bridge, and what went wrong setting it up.
    voice: (VoiceSetup, String),
    new_pin: Box<dyn FnMut() -> String + Send>,
}

impl Software {
    pub(crate) fn new(running: Version, new_pin: impl FnMut() -> String + Send + 'static) -> Self {
        Self {
            running,
            r3: R3 { check: Check::Never, release: None, error: None, install: None },
            firmware: Firmware { installed: String::new(), check: Check::Never, offer: None, error: None, install: None },
            web: Web::default(),
            uninstall: (UninstallState::Idle, String::new()),
            voice: (VoiceSetup::Unknown, String::new()),
            new_pin: Box::new(new_pin),
        }
    }

    /// The core is connected: learn what it knows.
    pub(crate) fn on_connect(&mut self, now: Instant) -> Vec<Command> {
        let mut commands = vec![Command::CheckFirmware { force: false }, Command::ReadWeb, Command::ReadAddress, Command::ReadVoice];
        commands.extend(self.tick(now));
        commands
    }

    /// Once a second: look for an r3cui update when it is time.
    pub(crate) fn tick(&mut self, now: Instant) -> Vec<Command> {
        if self.r3.install.is_none() && self.r3.check.due(now, R3_CHECK_EVERY) && !matches!(self.r3.check, Check::Done { ok: false, .. }) {
            self.r3.check = Check::Running;
            return vec![Command::CheckR3];
        }
        Vec::new()
    }

    pub(crate) fn act(&mut self, action: Action, now: Instant) -> Vec<Command> {
        match action {
            Action::Opened => {
                self.r3.error = None;
                self.firmware.error = None;
                if self.uninstall.0 == UninstallState::Failed {
                    self.uninstall = (UninstallState::Idle, String::new());
                }
                let mut commands = vec![Command::ReadWeb];
                if self.voice.0 != VoiceSetup::Working {
                    commands.push(Command::ReadVoice);
                }
                if self.web.address.is_empty() {
                    commands.push(Command::ReadAddress);
                }
                if self.r3.install.is_none() && self.r3.check.due(now, R3_CHECK_EVERY) {
                    self.r3.check = Check::Running;
                    commands.push(Command::CheckR3);
                }
                if self.firmware.install.is_none() && self.firmware.check.due(now, FIRMWARE_ASK_EVERY) {
                    self.firmware.check = Check::Running;
                    commands.push(Command::CheckFirmware { force: false });
                }
                commands
            }
            Action::CheckR3 if self.r3.install.is_none() && self.r3.check != Check::Running => {
                self.r3.check = Check::Running;
                vec![Command::CheckR3]
            }
            Action::InstallR3 => match self.r3.release.clone() {
                Some(release) if self.r3.install.is_none() => {
                    self.r3.error = None;
                    self.r3.install = Some((InstallPhase::Download, 0));
                    let access = self.access();
                    vec![Command::Go(PageId::R3Installing), Command::InstallR3 { release, access }]
                }
                _ => Vec::new(),
            },
            Action::CheckFirmware if self.firmware.install.is_none() && self.firmware.check != Check::Running => {
                self.firmware.check = Check::Running;
                self.firmware.error = None;
                vec![Command::CheckFirmware { force: true }]
            }
            Action::InstallFirmware => match &self.firmware.offer {
                Some(offer) if self.firmware.install.is_none() => {
                    let id = offer.id.clone();
                    self.firmware.error = None;
                    self.firmware.install = Some(FirmwareInstall { stage: Stage::Requested, phase: InstallPhase::Download, percent: 0, step: None });
                    vec![Command::StartFirmware { id }]
                }
                _ => Vec::new(),
            },
            Action::OpenWeb if self.web.enabled && self.web.pin.is_some() => Vec::new(),
            Action::OpenWeb | Action::NewPin => {
                let pin = (self.new_pin)();
                self.web.pin = Some(pin.clone());
                self.web.error = None;
                vec![Command::SetWeb { pin: Some(pin) }]
            }
            Action::WebOff => {
                self.web.enabled = false;
                self.web.pin = None;
                vec![Command::SetWeb { pin: None }]
            }
            Action::Uninstall if self.uninstall.0 != UninstallState::Working && !self.busy() => {
                self.uninstall = (UninstallState::Working, String::new());
                vec![Command::Uninstall { access: self.access() }]
            }
            Action::InstallVoice if self.voice.0 != VoiceSetup::Working && !self.busy() => {
                self.voice = (VoiceSetup::Working, String::new());
                vec![Command::InstallVoice { access: self.access() }]
            }
            _ => Vec::new(),
        }
    }

    fn busy(&self) -> bool {
        self.r3.install.is_some() || self.firmware.install.is_some() || self.uninstall.0 == UninstallState::Working
    }

    fn access(&mut self) -> Access {
        let known = self.web.enabled && self.web.pin.is_some();
        if !known {
            self.web.pin = Some((self.new_pin)());
        }
        Access { pin: self.web.pin.clone().unwrap_or_default(), enable: !known, restore_off: !self.web.enabled }
    }

    pub(crate) fn on_event(&mut self, event: Event, now: Instant) -> Vec<Command> {
        match event {
            Event::R3Checked(Ok(release)) => {
                self.r3.check = Check::Done { at: now, ok: true };
                self.r3.release = release;
                Vec::new()
            }
            Event::R3Checked(Err(e)) => {
                tracing::warn!("r3cui update check: {e}");
                self.r3.check = Check::Done { at: now, ok: false };
                Vec::new()
            }
            Event::R3Progress { phase, percent } => {
                self.r3.install = Some((phase, percent));
                Vec::new()
            }
            Event::R3Failed(why) => {
                self.r3.install = None;
                self.r3.error = Some(why);
                vec![Command::Go(PageId::R3Update)]
            }
            Event::UninstallFailed(why) => {
                self.uninstall = (UninstallState::Failed, why);
                Vec::new()
            }
            Event::Firmware(result) => self.on_firmware(result, now),
            Event::FirmwareUpdate(update) => self.on_firmware_update(update),
            Event::FirmwareStarted(result) => self.on_firmware_started(result),
            Event::WebRead(enabled) => {
                self.web.enabled = enabled;
                if !enabled {
                    self.web.pin = None;
                }
                Vec::new()
            }
            Event::WebSet(Ok(())) => {
                self.web.enabled = self.web.pin.is_some();
                Vec::new()
            }
            Event::WebSet(Err(e)) => {
                tracing::warn!("web configurator: {e}");
                self.web.enabled = false;
                self.web.pin = None;
                self.web.error = Some(e);
                Vec::new()
            }
            Event::Address(address) => {
                self.web.address = address;
                Vec::new()
            }
            Event::VoiceRead(read) => {
                if self.voice.0 != VoiceSetup::Working {
                    self.voice.0 = match read {
                        Ok(true) => VoiceSetup::Ready,
                        Ok(false) if self.voice.0 == VoiceSetup::Failed => VoiceSetup::Failed,
                        Ok(false) => VoiceSetup::Absent,
                        Err(e) => {
                            tracing::warn!("voice settings: {e}");
                            VoiceSetup::Unknown
                        }
                    };
                }
                Vec::new()
            }
            Event::VoiceInstalled(result) => {
                self.voice = match result {
                    Ok(()) => (VoiceSetup::Ready, String::new()),
                    Err(e) => {
                        tracing::warn!("setting up voice: {e}");
                        (VoiceSetup::Failed, e)
                    }
                };
                Vec::new()
            }
        }
    }

    fn on_firmware(&mut self, result: Result<SystemUpdateInfo, String>, now: Instant) -> Vec<Command> {
        let fw = &mut self.firmware;
        let info = match result {
            Ok(info) => info,
            Err(e) => {
                tracing::warn!("firmware update check: {e}");
                fw.check = Check::Done { at: now, ok: false };
                return Vec::new();
            }
        };
        fw.check = Check::Done { at: now, ok: true };
        fw.installed = info.installed_version;
        fw.offer = info
            .available
            .into_iter()
            .find(|u| u.version != fw.installed)
            .map(|u| Offer {
                notes: [crate::coreapi::localized(&u.title, NOTES_LANGUAGE), crate::coreapi::localized(&u.description, NOTES_LANGUAGE)].into_iter().flatten().collect::<Vec<_>>().join("\n"),
                id: u.id,
                version: u.version,
                release_date: u.release_date,
                size: u.size,
                download: u.download,
            });
        // The remote answers again after restarting into the new firmware.
        if !info.update_in_progress && fw.install.as_ref().is_some_and(|i| i.phase == InstallPhase::Restart) {
            fw.install = None;
            return vec![Command::Go(PageId::FwUpdate)];
        }
        let download = fw.offer.as_ref().map(|o| o.download);
        let id = fw.offer.as_ref().map(|o| o.id.clone());
        let stage = fw.install.as_ref().map(|i| i.stage);
        if stage.is_none() && info.update_in_progress {
            fw.install = Some(FirmwareInstall { stage: Stage::Running, phase: InstallPhase::Install, percent: 0, step: None });
            return vec![Command::Go(PageId::FwInstalling)];
        }
        match (stage, download, id) {
            (Some(Stage::Downloading), Some(DownloadState::Downloaded), Some(id)) => {
                if let Some(install) = &mut fw.install {
                    install.stage = Stage::Starting;
                }
                vec![Command::StartFirmware { id }]
            }
            (Some(Stage::Downloading), Some(DownloadState::Error), _) => self.fail_firmware("The download failed"),
            _ => Vec::new(),
        }
    }

    fn on_firmware_started(&mut self, result: Result<UpdateStart, UpdateRefusal>) -> Vec<Command> {
        let Some(install) = &mut self.firmware.install else { return Vec::new() };
        match result {
            Ok(UpdateStart::Download) => {
                install.stage = Stage::Downloading;
                install.phase = InstallPhase::Download;
                vec![Command::Go(PageId::FwInstalling)]
            }
            Ok(UpdateStart::Start | UpdateStart::Unknown) | Err(UpdateRefusal::Running) => {
                install.stage = Stage::Running;
                install.phase = InstallPhase::Install;
                vec![Command::Go(PageId::FwInstalling)]
            }
            Err(UpdateRefusal::Downloaded) if install.stage == Stage::Requested => {
                install.stage = Stage::Starting;
                match self.firmware.offer.as_ref() {
                    Some(offer) => vec![Command::StartFirmware { id: offer.id.clone() }],
                    None => Vec::new(),
                }
            }
            Err(UpdateRefusal::Downloaded) => self.fail_firmware("The remote would not start the installation"),
            Err(UpdateRefusal::BatteryLow) => self.fail_firmware(&format!("Battery too low: installing needs {BATTERY_NEEDED}% or more")),
            Err(UpdateRefusal::Other(e)) => self.fail_firmware(&e),
        }
    }

    fn fail_firmware(&mut self, why: &str) -> Vec<Command> {
        self.firmware.install = None;
        self.firmware.error = Some(why.to_string());
        vec![Command::Go(PageId::FwUpdate)]
    }

    fn on_firmware_update(&mut self, update: SoftwareUpdate) -> Vec<Command> {
        let mut commands = Vec::new();
        if self.firmware.install.is_none() && update.event_type != UpdateEventType::Stop {
            self.firmware.install = Some(FirmwareInstall { stage: Stage::Running, phase: InstallPhase::Install, percent: 0, step: None });
            commands.push(Command::Go(PageId::FwInstalling));
        }
        let Some(install) = &mut self.firmware.install else { return commands };
        match (update.event_type, update.progress) {
            (UpdateEventType::Stop, _) => match install.stage {
                Stage::Downloading => commands.push(Command::CheckFirmware { force: false }),
                Stage::Running => {
                    install.phase = InstallPhase::Restart;
                    install.percent = 100;
                }
                Stage::Requested | Stage::Starting => {}
            },
            (_, Some(p)) => match p.state {
                UpdateRunState::Download => {
                    install.phase = InstallPhase::Download;
                    install.percent = p.download_percent.unwrap_or(0).min(100);
                }
                UpdateRunState::Failure => return self.fail_firmware("The update failed"),
                UpdateRunState::Success | UpdateRunState::Done => match install.stage {
                    Stage::Downloading => commands.push(Command::CheckFirmware { force: false }),
                    _ => {
                        install.phase = InstallPhase::Restart;
                        install.percent = 100;
                    }
                },
                UpdateRunState::Start | UpdateRunState::Run | UpdateRunState::SubProcess | UpdateRunState::Progress => {
                    install.stage = Stage::Running;
                    install.phase = InstallPhase::Install;
                    if let (Some(step), Some(of)) = (p.current_step, p.total_steps.filter(|n| *n > 0)) {
                        install.step = Some((step, of));
                        install.percent = ((step.saturating_sub(1) * 100 + p.current_percent.unwrap_or(0).min(100) as u32) / of).min(100) as u8;
                    } else if let Some(percent) = p.current_percent {
                        install.percent = percent.min(100);
                    }
                }
                UpdateRunState::Idle | UpdateRunState::Unknown => {}
            },
            (_, None) => {}
        }
        commands
    }

    pub(crate) fn view(&self, now: Instant, battery: i32) -> View {
        let (r3, fw) = (&self.r3, &self.firmware);
        let has_r3 = r3.release.is_some();
        let new_r3 = r3.release.as_ref().map(|r| r.version.to_string()).unwrap_or_default();
        let (r3_phase, r3_percent) = r3.install.unwrap_or((InstallPhase::Idle, 0));
        let new_fw = fw.offer.as_ref().map(|o| o.version.clone()).unwrap_or_default();
        let (fw_phase, fw_percent, fw_step_line) = match &fw.install {
            None => (InstallPhase::Idle, 0, String::new()),
            Some(i) => {
                let line = match (i.phase, i.step) {
                    (InstallPhase::Install, Some((step, of))) => format!("Step {step} of {of} · {}%", i.percent),
                    (InstallPhase::Download, _) => format!("Downloading · {}%", i.percent),
                    (InstallPhase::Restart, _) => "Restarting".to_string(),
                    _ => format!("{}%", i.percent),
                };
                (i.phase, i.percent, line)
            }
        };
        let web_shown = self.web.enabled.then(|| self.web.pin.clone()).flatten();
        let configurator = (!self.web.address.is_empty()).then(|| format!("{}{CONFIGURATOR_PATH}", self.web.address));
        View {
            r3_version: Version::new(self.running.major, self.running.minor, self.running.patch).to_string() + &prerelease(&self.running),
            r3_build: Build::of(&self.running, crate::remote::selfupdate::COMMIT),
            r3_repo: REPO_PATH.to_string(),
            r3_state: r3.check.state(has_r3),
            r3_status: r3.check.line(now, has_r3, &new_r3),
            r3_short: r3.check.short(now, r3.release.as_ref()),
            r3_release_line: r3.release.as_ref().map(|r| release_line(&r.published_at, r.size)).unwrap_or_default(),
            r3_qr_text: r3.release.as_ref().map(|r| r.page_url.clone()).unwrap_or_default(),
            r3_new_version: new_r3,
            r3_phase,
            r3_percent,
            r3_error: r3.error.clone().unwrap_or_default(),
            uninstall_state: self.uninstall.0,
            uninstall_error: self.uninstall.1.clone(),
            voice_setup: self.voice.0,
            voice_line: match self.voice.0 {
                VoiceSetup::Unknown => String::new(),
                VoiceSetup::Absent => "Set up".into(),
                VoiceSetup::Working => "Setting up…".into(),
                VoiceSetup::Ready => "On".into(),
                VoiceSetup::Failed => "Failed · try again".into(),
            },
            fw_installed: fw.installed.clone(),
            fw_state: fw.check.state(fw.offer.is_some()),
            fw_status: fw.check.line(now, fw.offer.is_some(), &new_fw),
            fw_release_line: fw.offer.as_ref().map(|o| release_line(&o.release_date, o.size)).unwrap_or_default(),
            fw_notes: fw.offer.as_ref().map(|o| o.notes.clone()).unwrap_or_default(),
            fw_battery_line: format!("Battery {battery}% · installing needs {BATTERY_NEEDED}% or more"),
            fw_battery_ok: battery >= BATTERY_NEEDED,
            fw_error: fw.error.clone().unwrap_or_default(),
            fw_new_version: new_fw,
            fw_phase,
            fw_percent,
            fw_step_line,
            web_on: self.web.enabled,
            web_pin: web_shown.unwrap_or_default(),
            web_address: self.web.error.clone().or_else(|| configurator.clone()).unwrap_or_default(),
            web_qr_text: configurator.map(|c| format!("http://{c}")).unwrap_or_default(),
        }
    }
}

fn release_line(date: &str, size: u64) -> String {
    [release_date(date), (size > 0).then(|| size_label(size))].into_iter().flatten().collect::<Vec<_>>().join(" · ")
}

fn prerelease(v: &Version) -> String {
    if v.pre.is_empty() { String::new() } else { format!("-{}", v.pre) }
}

/// Which build this is, beside its version: the commit it came from (every build has one, releases too) and the
/// features it was built with, from after the `+` in its version (`g1eb1474.demo`), where the commit is skipped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Build {
    pub(crate) commit: String,
    pub(crate) features: String,
}

impl Build {
    fn of(v: &Version, commit: &str) -> Self {
        let is_commit = |p: &&str| p.strip_prefix('g').is_some_and(|h| !h.is_empty() && h.chars().all(|c| c.is_ascii_hexdigit()));
        let features = v.build.as_str().split('.').filter(|p| !p.is_empty() && !is_commit(p)).collect::<Vec<_>>().join(" · ");
        Self { commit: commit.to_string(), features }
    }
}

/// Everything the System pages show, as plain values for `view::software::show`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct View {
    /// The release it is (or is built from), without the build after a `+`.
    pub(crate) r3_version: String,
    pub(crate) r3_build: Build,
    /// Under the update tile's title: "6.8 MB", "2 min ago".
    pub(crate) r3_short: String,
    pub(crate) r3_repo: String,
    pub(crate) r3_state: UpdateState,
    pub(crate) r3_status: String,
    pub(crate) r3_new_version: String,
    pub(crate) r3_release_line: String,
    pub(crate) r3_qr_text: String,
    pub(crate) r3_phase: InstallPhase,
    pub(crate) r3_percent: u8,
    pub(crate) r3_error: String,
    pub(crate) uninstall_state: UninstallState,
    pub(crate) uninstall_error: String,
    pub(crate) voice_setup: VoiceSetup,
    pub(crate) voice_line: String,
    pub(crate) fw_installed: String,
    pub(crate) fw_state: UpdateState,
    pub(crate) fw_status: String,
    pub(crate) fw_new_version: String,
    pub(crate) fw_release_line: String,
    pub(crate) fw_notes: String,
    pub(crate) fw_battery_line: String,
    pub(crate) fw_battery_ok: bool,
    pub(crate) fw_error: String,
    pub(crate) fw_phase: InstallPhase,
    pub(crate) fw_percent: u8,
    pub(crate) fw_step_line: String,
    pub(crate) web_on: bool,
    pub(crate) web_pin: String,
    pub(crate) web_address: String,
    pub(crate) web_qr_text: String,
}

/// A fresh four-digit PIN. Not secret-grade randomness: the PIN guards a LAN-only page and is replaced at will.
pub(crate) fn random_pin() -> String {
    use std::hash::{BuildHasher, Hasher};
    let n = std::collections::hash_map::RandomState::new().build_hasher().finish();
    format!("{:04}", n % 10_000)
}

/// Starts what `commands` ask for; each answers later as `Later::Software`.
pub(crate) fn run(commands: Vec<Command>, link: &Link, later: &UnboundedSender<Later>, weak: &slint::Weak<AppWindow>) {
    for command in commands {
        let (link, later) = (link.clone(), later.clone());
        let tell = {
            let later = later.clone();
            move |event: Event| drop(later.send(Later::Software(event)))
        };
        match command {
            Command::Go(page) => ui(weak, move |app| app.go_to(page, 0)),
            Command::CheckR3 => {
                tokio::spawn(async move { tell(Event::R3Checked(link.latest_release().await)) });
            }
            Command::InstallR3 { release, access } => {
                tokio::spawn(async move {
                    let report = tell.clone();
                    let progress: Progress = Arc::new(move |phase, percent| report(Event::R3Progress { phase, percent }));
                    if let Err(e) = link.install_r3(release, access, progress).await {
                        tracing::warn!("installing r3cui: {e}");
                        tell(Event::R3Failed(e));
                    }
                });
            }
            Command::CheckFirmware { force } => {
                tokio::spawn(async move { tell(Event::Firmware(link.firmware_info(force).await)) });
            }
            Command::StartFirmware { id } => {
                tokio::spawn(async move { tell(Event::FirmwareStarted(link.start_firmware(&id, &later).await)) });
            }
            Command::ReadWeb => {
                tokio::spawn(async move {
                    match link.web_enabled().await {
                        Ok(enabled) => tell(Event::WebRead(enabled)),
                        Err(e) => tracing::warn!("web configurator state: {e}"),
                    }
                });
            }
            Command::ReadAddress => {
                tokio::spawn(async move { tell(Event::Address(link.address().await)) });
            }
            Command::SetWeb { pin } => {
                tokio::spawn(async move { tell(Event::WebSet(link.set_web(pin.as_deref()).await)) });
            }
            Command::Uninstall { access } => {
                tokio::spawn(async move {
                    if let Err(e) = link.uninstall(access).await {
                        tracing::warn!("uninstalling r3cui: {e}");
                        tell(Event::UninstallFailed(e));
                    }
                });
            }
            Command::ReadVoice => {
                tokio::spawn(async move { tell(Event::VoiceRead(link.voice_ready().await)) });
            }
            Command::InstallVoice { access } => {
                tokio::spawn(async move { tell(Event::VoiceInstalled(link.install_voice(access).await)) });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_shows_its_release_large_and_the_build_under_it() {
        let v = |s| Version::parse(s).unwrap();
        let build = |s| Build::of(&v(s), "1eb1474");
        assert_eq!(build("0.2.0+g1eb1474.demo.lab"), Build { commit: "1eb1474".into(), features: "demo · lab".into() });
        assert_eq!(build("0.2.0+demo"), Build { commit: "1eb1474".into(), features: "demo".into() }, "a release built with features");
        assert_eq!(build("0.2.0"), Build { commit: "1eb1474".into(), features: String::new() }, "a release shows its commit too");
        let view = Software::new(v("0.2.0-rc.1+g1eb1474"), || "0000".into()).view(Instant::now(), 100);
        assert_eq!((view.r3_version.as_str(), view.r3_build.commit.as_str()), ("0.2.0-rc.1", crate::remote::selfupdate::COMMIT));
    }
    use crate::coreapi::{AvailableUpdate, UpdateProgress};

    fn software() -> Software {
        let mut n = 0;
        Software::new(Version::new(0, 1, 0), move || {
            n += 1;
            format!("{n:04}")
        })
    }

    fn release(version: &str) -> Release {
        Release {
            version: Version::parse(version).unwrap(),
            page_url: format!("https://github.com/jackhric/r3cui/releases/tag/v{version}"),
            published_at: "2026-11-20T09:00:00Z".into(),
            size: 6_800_000,
            download_url: "https://example.invalid/r3cui-0.2.0.tar.gz".into(),
            sha256: "ab".into(),
        }
    }

    fn firmware_info(download: DownloadState, in_progress: bool) -> SystemUpdateInfo {
        SystemUpdateInfo {
            update_in_progress: in_progress,
            last_check_date: None,
            installed_version: "2.8.4".into(),
            available: vec![AvailableUpdate {
                id: "u1".into(),
                title: [("en".to_string(), "Firmware 2.9.0".to_string())].into(),
                description: [("en".to_string(), "Faster Wi-Fi.".to_string())].into(),
                version: "2.9.0".into(),
                channel: String::new(),
                release_date: "2026-11-10T00:00:00Z".into(),
                size: 118_000_000,
                release_notes_url: None,
                download,
            }],
        }
    }

    fn progress(state: UpdateRunState, step: u32, of: u32, percent: u8) -> SoftwareUpdate {
        SoftwareUpdate {
            event_type: UpdateEventType::Progress,
            update_id: "u1".into(),
            progress: Some(UpdateProgress { state, download_percent: Some(percent), total_steps: Some(of), current_step: Some(step), current_percent: Some(percent) }),
        }
    }

    #[test]
    fn the_first_tick_looks_for_an_update_and_a_result_is_not_asked_again_for_a_day() {
        let (mut s, now) = (software(), Instant::now());
        assert_eq!(s.tick(now), [Command::CheckR3]);
        assert_eq!(s.tick(now), [], "one check at a time");
        s.on_event(Event::R3Checked(Ok(Some(release("0.2.0")))), now);
        let v = s.view(now + Duration::from_secs(120), 80);
        assert_eq!((v.r3_state, v.r3_status.as_str(), v.r3_new_version.as_str()), (UpdateState::Available, "Update available · 0.2.0", "0.2.0"));
        assert_eq!(v.r3_release_line, "Nov 20, 2026 · 6.8 MB");
        assert_eq!(s.tick(now + Duration::from_secs(3600)), []);
        assert_eq!(s.tick(now + R3_CHECK_EVERY), [Command::CheckR3]);
    }

    #[test]
    fn a_current_check_reads_as_up_to_date_and_a_failed_one_is_not_retried_by_the_clock() {
        let (mut s, now) = (software(), Instant::now());
        s.tick(now);
        s.on_event(Event::R3Checked(Ok(None)), now);
        let v = s.view(now + Duration::from_secs(150), 80);
        assert_eq!((v.r3_state, v.r3_status.as_str()), (UpdateState::Current, "Up to date · checked 2 min ago"));
        let mut s = software();
        s.tick(now);
        s.on_event(Event::R3Checked(Err("offline".into())), now);
        assert_eq!(s.view(now, 80).r3_state, UpdateState::Failed);
        assert_eq!(s.tick(now + Duration::from_secs(5)), [], "a failed check waits for the System page or the user");
        assert!(s.act(Action::Opened, now).contains(&Command::CheckR3));
    }

    #[test]
    fn installing_r3cui_gets_the_web_configurator_a_pin_and_puts_it_back() {
        let (mut s, now) = (software(), Instant::now());
        s.tick(now);
        s.on_event(Event::R3Checked(Ok(Some(release("0.2.0")))), now);
        let commands = s.act(Action::InstallR3, now);
        let access = Access { pin: "0001".into(), enable: true, restore_off: true };
        assert_eq!(commands, [Command::Go(PageId::R3Installing), Command::InstallR3 { release: release("0.2.0"), access }]);
        assert_eq!(s.view(now, 80).r3_phase, InstallPhase::Download);
        s.on_event(Event::R3Progress { phase: InstallPhase::Install, percent: 80 }, now);
        assert_eq!((s.view(now, 80).r3_phase, s.view(now, 80).r3_percent), (InstallPhase::Install, 80));
        assert_eq!(s.act(Action::InstallR3, now), [], "not twice at once");
        assert_eq!(s.on_event(Event::R3Failed("disk full".into()), now), [Command::Go(PageId::R3Update)]);
        let v = s.view(now, 80);
        assert_eq!((v.r3_phase, v.r3_error.as_str()), (InstallPhase::Idle, "disk full"));
    }

    #[test]
    fn a_configurator_that_is_on_with_a_known_pin_is_used_as_it_is() {
        let (mut s, now) = (software(), Instant::now());
        assert_eq!(s.act(Action::OpenWeb, now), [Command::SetWeb { pin: Some("0001".into()) }]);
        s.on_event(Event::WebSet(Ok(())), now);
        assert_eq!(s.act(Action::OpenWeb, now), [], "already on with a PIN to show");
        let v = s.view(now, 80);
        assert_eq!((v.web_on, v.web_pin.as_str()), (true, "0001"));
        s.on_event(Event::Address("192.168.1.40".into()), now);
        let v = s.view(now, 80);
        assert_eq!((v.web_address.as_str(), v.web_qr_text.as_str()), ("192.168.1.40/configurator", "http://192.168.1.40/configurator"));
        s.on_event(Event::R3Checked(Ok(Some(release("0.2.0")))), now);
        let Some(Command::InstallR3 { access, .. }) = s.act(Action::InstallR3, now).pop() else { panic!("no install") };
        assert_eq!(access, Access { pin: "0001".into(), enable: false, restore_off: false });
        assert_eq!(s.act(Action::NewPin, now), [Command::SetWeb { pin: Some("0002".into()) }]);
        assert_eq!(s.act(Action::WebOff, now), [Command::SetWeb { pin: None }]);
        let v = s.view(now, 80);
        assert_eq!((v.web_on, v.web_pin.as_str()), (false, ""));
    }

    #[test]
    fn a_pin_is_four_digits() {
        for _ in 0..50 {
            let pin = random_pin();
            assert!(pin.len() == 4 && pin.chars().all(|c| c.is_ascii_digit()), "{pin}");
        }
    }

    #[test]
    fn uninstalling_asks_for_access_and_can_fail() {
        let (mut s, now) = (software(), Instant::now());
        let commands = s.act(Action::Uninstall, now);
        assert_eq!(commands, [Command::Uninstall { access: Access { pin: "0001".into(), enable: true, restore_off: true } }]);
        assert_eq!(s.view(now, 80).uninstall_state, UninstallState::Working);
        assert_eq!(s.act(Action::Uninstall, now), [], "once");
        s.on_event(Event::UninstallFailed("no PIN".into()), now);
        let v = s.view(now, 80);
        assert_eq!((v.uninstall_state, v.uninstall_error.as_str()), (UninstallState::Failed, "no PIN"));
        s.act(Action::Opened, now);
        assert_eq!(s.view(now, 80).uninstall_state, UninstallState::Idle);
    }

    #[test]
    fn a_firmware_update_is_offered_with_its_notes_and_the_battery_it_needs() {
        let (mut s, now) = (software(), Instant::now());
        s.on_event(Event::Firmware(Ok(firmware_info(DownloadState::Pending, false))), now);
        let v = s.view(now, 82);
        assert_eq!((v.fw_state, v.fw_installed.as_str(), v.fw_new_version.as_str()), (UpdateState::Available, "2.8.4", "2.9.0"));
        assert_eq!((v.fw_notes.as_str(), v.fw_release_line.as_str()), ("Firmware 2.9.0\nFaster Wi-Fi.", "Nov 10, 2026 · 118 MB"));
        assert_eq!((v.fw_battery_line.as_str(), v.fw_battery_ok), ("Battery 82% · installing needs 50% or more", true));
        assert!(!s.view(now, 49).fw_battery_ok);
    }

    #[test]
    fn firmware_is_downloaded_then_installed_through_to_the_restart() {
        let (mut s, now) = (software(), Instant::now());
        s.on_event(Event::Firmware(Ok(firmware_info(DownloadState::Pending, false))), now);
        assert_eq!(s.act(Action::InstallFirmware, now), [Command::StartFirmware { id: "u1".into() }]);
        assert_eq!(s.on_event(Event::FirmwareStarted(Ok(UpdateStart::Download)), now), [Command::Go(PageId::FwInstalling)]);

        let mut download = progress(UpdateRunState::Download, 0, 0, 40);
        download.progress.as_mut().unwrap().download_percent = Some(40);
        s.on_event(Event::FirmwareUpdate(download), now);
        let v = s.view(now, 82);
        assert_eq!((v.fw_phase, v.fw_percent, v.fw_step_line.as_str()), (InstallPhase::Download, 40, "Downloading · 40%"));

        let stop = SoftwareUpdate { event_type: UpdateEventType::Stop, update_id: "u1".into(), progress: None };
        assert_eq!(s.on_event(Event::FirmwareUpdate(stop), now), [Command::CheckFirmware { force: false }]);
        assert_eq!(s.on_event(Event::Firmware(Ok(firmware_info(DownloadState::Downloaded, false))), now), [Command::StartFirmware { id: "u1".into() }]);
        s.on_event(Event::FirmwareStarted(Ok(UpdateStart::Start)), now);

        s.on_event(Event::FirmwareUpdate(progress(UpdateRunState::Run, 2, 4, 50)), now);
        let v = s.view(now, 82);
        assert_eq!((v.fw_phase, v.fw_percent, v.fw_step_line.as_str()), (InstallPhase::Install, 37, "Step 2 of 4 · 37%"));
        s.on_event(Event::FirmwareUpdate(progress(UpdateRunState::Done, 4, 4, 100)), now);
        assert_eq!(s.view(now, 82).fw_phase, InstallPhase::Restart);
        let mut after = firmware_info(DownloadState::Pending, false);
        after.installed_version = "2.9.0".into();
        after.available.clear();
        assert_eq!(s.on_event(Event::Firmware(Ok(after)), now), [Command::Go(PageId::FwUpdate)]);
        let v = s.view(now, 82);
        assert_eq!((v.fw_phase, v.fw_installed.as_str(), v.fw_state), (InstallPhase::Idle, "2.9.0", UpdateState::Current));
    }

    #[test]
    fn firmware_that_cannot_start_says_why_and_returns_to_its_page() {
        let (mut s, now) = (software(), Instant::now());
        s.on_event(Event::Firmware(Ok(firmware_info(DownloadState::Pending, false))), now);
        s.act(Action::InstallFirmware, now);
        assert_eq!(s.on_event(Event::FirmwareStarted(Err(UpdateRefusal::BatteryLow)), now), [Command::Go(PageId::FwUpdate)]);
        let v = s.view(now, 30);
        assert_eq!((v.fw_phase, v.fw_error.as_str()), (InstallPhase::Idle, "Battery too low: installing needs 50% or more"));
        s.act(Action::InstallFirmware, now);
        s.on_event(Event::FirmwareStarted(Ok(UpdateStart::Download)), now);
        assert_eq!(s.on_event(Event::FirmwareUpdate(progress(UpdateRunState::Failure, 0, 0, 0)), now), [Command::Go(PageId::FwUpdate)]);
    }

    #[test]
    fn an_update_already_running_takes_the_screen() {
        let (mut s, now) = (software(), Instant::now());
        assert_eq!(s.on_event(Event::Firmware(Ok(firmware_info(DownloadState::Downloaded, true))), now), [Command::Go(PageId::FwInstalling)]);
        assert_eq!(s.view(now, 80).fw_phase, InstallPhase::Install);
        let mut other = software();
        let start = SoftwareUpdate { event_type: UpdateEventType::Start, update_id: "u1".into(), progress: None };
        assert_eq!(other.on_event(Event::FirmwareUpdate(start), now), [Command::Go(PageId::FwInstalling)]);
    }
}
