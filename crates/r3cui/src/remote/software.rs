//! The remote's software through the core: firmware updates, the web configurator, and putting r3cui's own updates on
//! the remote or taking r3cui off. What to show of it is `app::software`.

use crate::app::software::Access;
use crate::remote::rest::Rest;
use crate::remote::selfupdate::{self, Release};
use crate::InstallPhase;
use crate::coreapi::{CheckSystemUpdate, Core, GetVersion, SystemUpdateInfo, UpdateRefusal, UpdateStart};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Where an install is, as a share of the whole: the download first, then the archive going to the core.
const DOWNLOAD_SHARE: u64 = 70;
const INSTALL_SHARE: u8 = 95;
/// The core restarts the UI as soon as the install is in; this much later it did not.
const RESTART_WAIT: Duration = Duration::from_secs(90);
/// Left in the data folder while the web configurator is on only for an install, so that a UI that starts afterwards
/// can switch it off again if this one did not get to.
const RESTORE_MARKER: &str = "web-configurator-was-off";
const UPDATE_FOLDER: &str = "update";
const CONFIG_HOME_ENV: &str = "UC_CONFIG_HOME";
const DATA_HOME_ENV: &str = "UC_DATA_HOME";

pub(crate) type Progress = Arc<dyn Fn(InstallPhase, u8) + Send + Sync>;

pub(crate) async fn firmware_info(core: &Core, force: bool) -> Result<SystemUpdateInfo, String> {
    core.call(&CheckSystemUpdate { force_update: force }).await.map_err(|e| e.to_string())
}

pub(crate) async fn start_firmware(core: &Core, id: &str) -> Result<UpdateStart, UpdateRefusal> {
    core.update_system(id).await
}

pub(crate) async fn web_enabled(core: &Core) -> Result<bool, String> {
    core.api_access().await.map(|a| a.web_configurator.enabled).map_err(|e| e.to_string())
}

pub(crate) async fn set_web(core: &Core, pin: Option<&str>) -> Result<(), String> {
    match pin {
        Some(pin) => core.enable_web_configurator(pin).await,
        None => core.disable_web_configurator().await,
    }
    .map_err(|e| e.to_string())
}

/// The remote's address on the network, for the web configurator's link.
pub(crate) async fn address(core: &Core) -> String {
    match core.call(&GetVersion {}).await {
        Ok(v) if !v.address.is_empty() => v.address,
        Ok(v) => v.hostname,
        Err(e) => {
            tracing::warn!("remote address: {e}");
            String::new()
        }
    }
}

/// The latest release if it is newer than this one.
pub(crate) async fn latest_release() -> Result<Option<Release>, String> {
    let fetched = tokio::task::spawn_blocking(selfupdate::fetch_latest).await.map_err(|e| e.to_string())?;
    fetched.map(|r| selfupdate::newer(r, &selfupdate::current_version())).map_err(|e| format!("{e:#}"))
}

fn folder(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(PathBuf::from)
}

fn data_folder() -> PathBuf {
    folder(DATA_HOME_ENV).unwrap_or_else(|| std::env::temp_dir().join("r3cui"))
}

pub(crate) fn rest(access: &Access) -> Rest {
    Rest::new(&crate::coreapi::Config::url_from_env(), &access.pin)
}

pub(crate) async fn grant(core: &Core, access: &Access) -> Result<(), String> {
    if !access.enable {
        return Ok(());
    }
    if access.restore_off {
        if let Err(e) = std::fs::create_dir_all(data_folder()).and_then(|()| std::fs::write(data_folder().join(RESTORE_MARKER), b"")) {
            tracing::warn!("{RESTORE_MARKER}: {e}");
        }
    }
    set_web(core, Some(&access.pin)).await.map_err(|e| format!("Could not switch the web configurator on: {e}"))
}

/// Puts the web configurator back as it was before `grant`.
pub(crate) async fn revoke(core: &Core, access: &Access) {
    if !access.restore_off {
        return;
    }
    match set_web(core, None).await {
        Ok(()) => {
            let _ = std::fs::remove_file(data_folder().join(RESTORE_MARKER));
        }
        Err(e) => tracing::warn!("switching the web configurator off again: {e}"),
    }
}

/// At start-up: switch the web configurator off if an install left it on.
pub(crate) async fn finish_pending_revoke(core: &Core) {
    let marker = data_folder().join(RESTORE_MARKER);
    if marker.exists() && set_web(core, None).await.is_ok() {
        let _ = std::fs::remove_file(marker);
    }
}

/// Downloads `release`, puts it on the remote as its UI and waits to be restarted into it. Only an `Err` ends in this
/// function: when it works, the core stops this process.
pub(crate) async fn install_r3(core: &Core, release: Release, access: Access, progress: Progress) -> Result<(), String> {
    grant(core, &access).await?;
    let archive = data_folder().join(UPDATE_FOLDER).join(selfupdate::asset_name(&release.version));
    let sent = send_archive(&release, &archive, &access, &progress).await;
    let _ = std::fs::remove_file(&archive);
    revoke(core, &access).await;
    sent?;
    progress(InstallPhase::Restart, INSTALL_SHARE);
    tokio::time::sleep(RESTART_WAIT).await;
    Err("The remote did not restart into the new version".into())
}

async fn send_archive(release: &Release, archive: &Path, access: &Access, progress: &Progress) -> Result<(), String> {
    let (rel, path, report) = (release.clone(), archive.to_path_buf(), progress.clone());
    tokio::task::spawn_blocking(move || {
        selfupdate::download(&rel, &path, |done, total| report(InstallPhase::Download, (DOWNLOAD_SHARE * done / total.max(1)).min(DOWNLOAD_SHARE) as u8))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{e:#}"))?;
    progress(InstallPhase::Install, DOWNLOAD_SHARE as u8);
    let (rest, path) = (rest(access), archive.to_path_buf());
    tokio::task::spawn_blocking(move || rest.install(&path)).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())
}

/// Erases r3cui's own files and has the core remove it. Only an `Err` ends in this function: when it works, the core
/// restarts into the stock UI and stops this process.
pub(crate) async fn uninstall(core: &Core, access: Access) -> Result<(), String> {
    grant(core, &access).await?;
    let result = remove_ui(&access).await;
    revoke(core, &access).await;
    result?;
    tokio::time::sleep(RESTART_WAIT).await;
    Err("The remote did not restart into its stock interface".into())
}

async fn remove_ui(access: &Access) -> Result<(), String> {
    let rest = Arc::new(rest(access));
    let checked = rest.clone();
    // Erasing comes before removing, so nothing is erased unless the core is known to take the PIN.
    tokio::task::spawn_blocking(move || checked.check()).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        let folders: Vec<PathBuf> = [CONFIG_HOME_ENV, DATA_HOME_ENV].into_iter().filter_map(folder).collect();
        let failed = selfupdate::wipe_own_files(&folders.iter().map(PathBuf::as_path).collect::<Vec<_>>());
        failed.iter().for_each(|f| tracing::warn!("not erased: {f}"));
        rest.uninstall()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}
