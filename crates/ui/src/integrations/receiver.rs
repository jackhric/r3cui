//! The receiver, as Home Assistant reports it: its volume scale and power state.

use serde_json::Value;

/// The Denon tops out at 98 (+18 dB).
pub(crate) const MAX_VOLUME: i32 = 98;
/// Whether the remote is on its charger, from any message that says.
pub(crate) fn on_power(v: &Value) -> Option<bool> {
    // The simulator is always on its charger; previews of it can pretend otherwise.
    if cfg!(feature = "preview") && std::env::var_os("UC_PREVIEW_ON_BATTERY").is_some() {
        return Some(false);
    }
    v.get("power_supply").or_else(|| v.pointer("/battery/power_supply")).and_then(Value::as_bool)
}
/// The strip is 100 steps from silence to the receiver's limit; never less than one level a step.
pub(crate) fn slide(from: i32, steps: i32, max: i32) -> i32 {
    let by = (steps as f64 * max as f64 / 100.0).round() as i32;
    let by = if by == 0 { steps.signum() } else { by };
    (from + by).clamp(0, max)
}
pub(crate) fn volume_percent(volume: i32, max: i32) -> i32 {
    ((volume.max(0) as f64 / max.max(1) as f64) * 100.0).round().min(100.0) as i32
}
/// Denon reports volume as dB + 80, so the receiver's own dB reading is the level minus 80.
pub(crate) fn volume_db(volume: i32) -> String {
    match volume - 80 {
        db if db < 0 => format!("−{} dB", -db),
        0 => "0 dB".to_string(),
        db => format!("+{db} dB"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_scale() {
        assert_eq!(volume_db(62), "−18 dB");
        assert_eq!(volume_db(80), "0 dB");
        assert_eq!(volume_percent(72, 72), 100);
        assert_eq!(volume_percent(36, 72), 50);
        assert_eq!(volume_percent(80, 72), 100);
        assert_eq!(slide(0, 100, 72), 72);
        assert_eq!(slide(36, 1, 72), 37);
        assert_eq!(slide(36, -50, 72), 0);
    }
}
