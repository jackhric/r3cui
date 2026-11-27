//! The demo's software: a firmware update and an r3cui update waiting, and installs that advance on a clock. Nothing
//! touches the network or the REST API; the made-up values are `demo::data`.

use crate::app::software::Event;
use crate::app::Later;
use crate::remote::selfupdate::Release;
use crate::remote::software::Progress;
use crate::InstallPhase;
use coreapi::{AvailableUpdate, DownloadState, SoftwareUpdate, SystemUpdateInfo, UpdateEventType, UpdateProgress, UpdateRefusal, UpdateRunState, UpdateStart};
use demo::data::*;
use semver::Version;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

const TICK: Duration = Duration::from_millis(250);
const VOICE_SETUP_TIME: Duration = Duration::from_secs(3);
const DOWNLOAD_SHARE: u8 = 70;
const INSTALL_SHARE: u8 = 95;

#[derive(Clone, Default)]
pub(crate) struct Updates(Arc<Mutex<State>>);

#[derive(Default)]
struct State {
    updated: bool,
    downloaded: bool,
    web_on: bool,
    voice: bool,
}

impl Updates {
    pub(crate) fn firmware_info(&self) -> SystemUpdateInfo {
        let state = self.0.lock().unwrap();
        let text = |s: &str| [("en".to_string(), s.to_string())].into();
        SystemUpdateInfo {
            update_in_progress: false,
            last_check_date: None,
            installed_version: if state.updated { FIRMWARE_UPDATE } else { FIRMWARE_INSTALLED }.into(),
            available: if state.updated {
                Vec::new()
            } else {
                vec![AvailableUpdate {
                    id: FIRMWARE_UPDATE_ID.into(),
                    title: text(&format!("Firmware {FIRMWARE_UPDATE}")),
                    description: text(FIRMWARE_NOTES),
                    version: FIRMWARE_UPDATE.into(),
                    channel: String::new(),
                    release_date: FIRMWARE_RELEASE_DATE.into(),
                    size: FIRMWARE_SIZE,
                    release_notes_url: None,
                    download: if state.downloaded { DownloadState::Downloaded } else { DownloadState::Pending },
                }]
            },
        }
    }

    /// The first call fetches the update, the second installs it, as the core does.
    pub(crate) fn start_firmware(&self, later: &UnboundedSender<Later>) -> Result<UpdateStart, UpdateRefusal> {
        let downloaded = self.0.lock().unwrap().downloaded;
        let (updates, later) = (self.clone(), later.clone());
        if downloaded {
            tokio::spawn(install(updates, later));
            return Ok(UpdateStart::Start);
        }
        tokio::spawn(async move {
            emit(&later, update(UpdateEventType::Start, None));
            for percent in (0..=100).step_by(5) {
                emit(&later, update(UpdateEventType::Progress, Some(progress(UpdateRunState::Download, 0, percent))));
                tokio::time::sleep(DOWNLOAD_TIME / 20).await;
            }
            updates.0.lock().unwrap().downloaded = true;
            emit(&later, update(UpdateEventType::Stop, None));
        });
        Ok(UpdateStart::Download)
    }

    pub(crate) fn web_enabled(&self) -> bool {
        self.0.lock().unwrap().web_on
    }

    pub(crate) fn set_web(&self, on: bool) {
        self.0.lock().unwrap().web_on = on;
    }

    pub(crate) fn voice_ready(&self) -> bool {
        self.0.lock().unwrap().voice
    }
}

fn emit(later: &UnboundedSender<Later>, update: SoftwareUpdate) {
    let _ = later.send(Later::Software(Event::FirmwareUpdate(update)));
}

/// The installation's steps, then the restart, after which the remote reports the new version.
async fn install(updates: Updates, later: UnboundedSender<Later>) {
    const SLICES: u32 = 5;
    for step in 1..=FIRMWARE_STEPS {
        for slice in 0..SLICES {
            emit(&later, update(UpdateEventType::Progress, Some(progress(UpdateRunState::Run, step, (100 * slice / SLICES) as u8))));
            tokio::time::sleep(INSTALL_TIME / FIRMWARE_STEPS / SLICES).await;
        }
    }
    emit(&later, update(UpdateEventType::Progress, Some(progress(UpdateRunState::Done, FIRMWARE_STEPS, 100))));
    tokio::time::sleep(RESTART_TIME).await;
    {
        let mut state = updates.0.lock().unwrap();
        state.updated = true;
        state.downloaded = false;
    }
    let _ = later.send(Later::Software(Event::Firmware(Ok(updates.firmware_info()))));
}

fn update(event_type: UpdateEventType, progress: Option<UpdateProgress>) -> SoftwareUpdate {
    SoftwareUpdate { event_type, update_id: FIRMWARE_UPDATE_ID.into(), progress }
}

fn progress(state: UpdateRunState, step: u32, percent: u8) -> UpdateProgress {
    UpdateProgress { state, download_percent: Some(percent), total_steps: Some(FIRMWARE_STEPS), current_step: Some(step), current_percent: Some(percent) }
}

/// The next minor release after the build that runs, so the demo always has an update to offer.
pub(crate) fn release() -> Release {
    let running = crate::remote::selfupdate::current_version();
    let version = Version::new(running.major, running.minor + 1, 0);
    Release {
        page_url: format!("https://{}/releases/tag/v{version}", crate::remote::selfupdate::REPO_PATH),
        version,
        published_at: R3_RELEASE_DATE.into(),
        size: R3_SIZE,
        download_url: String::new(),
        sha256: String::new(),
    }
}

pub(crate) async fn install_r3(progress: Progress) -> Result<(), String> {
    let ticks = |time: Duration| (time.as_millis() / TICK.as_millis()).max(1) as u32;
    let n = ticks(DOWNLOAD_TIME);
    for i in 0..=n {
        progress(InstallPhase::Download, (DOWNLOAD_SHARE as u32 * i / n) as u8);
        tokio::time::sleep(TICK).await;
    }
    let n = ticks(INSTALL_TIME);
    for i in 0..=n {
        progress(InstallPhase::Install, DOWNLOAD_SHARE + ((INSTALL_SHARE - DOWNLOAD_SHARE) as u32 * i / n) as u8);
        tokio::time::sleep(TICK).await;
    }
    progress(InstallPhase::Restart, INSTALL_SHARE);
    tokio::time::sleep(RESTART_TIME).await;
    Err("Demo: nothing was installed. The remote would restart here.".into())
}

pub(crate) async fn uninstall() -> Result<(), String> {
    tokio::time::sleep(RESTART_TIME).await;
    Err("Demo: nothing was erased. The remote would restart into its stock interface.".into())
}

/// The demo's voice integration goes in at once, made up like the rest.
pub(crate) async fn install_voice(updates: &Updates) -> Result<(), String> {
    tokio::time::sleep(VOICE_SETUP_TIME).await;
    updates.0.lock().unwrap().voice = true;
    Ok(())
}
