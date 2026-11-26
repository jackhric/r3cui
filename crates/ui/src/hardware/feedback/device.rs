const HAPTIC_DEV_PATH_ENV: &str = "UC_HAPTIC_DEV_PATH";

pub(super) fn write_effect(id: u8) {
    let Some(path) = std::env::var_os(HAPTIC_DEV_PATH_ENV) else { return };
    if let Err(e) = std::fs::write(&path, id.to_string()) {
        tracing::debug!("haptic effect {id}: {e}");
    }
}
