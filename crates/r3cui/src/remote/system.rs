//! The remote itself, as the core reports it: battery, charger, whether it's awake, and the display, haptics, sound
//! and power-saving settings the menu shows.

use slint::ComponentHandle;
use crate::Prefs;
use crate::view::format::duration_label;
use crate::view::ui;
use crate::hardware::feedback;
use crate::AppWindow;
use coreapi::{BatteryStatus, PowerMode, PowerModeChange};
use serde_json::{json, Value};

pub(crate) struct System {
    pub(crate) battery: i32,
    pub(crate) charging: bool,
    pub(crate) awake: bool,
    /// Whether to go dark when the remote sleeps. A desktop build looks at a remote that is usually asleep on its
    /// dock, so it only follows the remote's power state when asked to.
    follow_power: bool,
}

impl System {
    pub(crate) async fn load(core: &coreapi::Core) -> Self {
        let (power, charger) = tokio::join!(core.fetch("get_power_mode"), core.fetch("get_battery_charger"));
        let (power, charger) = (power.unwrap_or(Value::Null), charger.unwrap_or(Value::Null));
        tracing::info!(%power, %charger, "power");
        let battery = power.pointer("/battery/capacity").or(charger.get("capacity")).and_then(Value::as_i64).unwrap_or(100) as i32;
        let supply = |v: &Value| v.get("power_supply").or_else(|| v.pointer("/battery/power_supply")).and_then(Value::as_bool);
        let follow_power = std::env::var("UC_MODEL").is_ok_and(|m| m == "UCR3") || std::env::var_os("UC_FOLLOW_POWER").is_some();
        let mut s = Self { battery, charging: on_charger(supply(&power).or_else(|| supply(&charger))).unwrap_or(false), awake: true, follow_power };
        s.awake = s.is_awake(power.get("mode").and_then(|m| serde_json::from_value(m.clone()).ok()));
        s
    }

    #[cfg(feature = "demo")]
    pub(crate) fn demo() -> Self {
        Self { battery: demo::data::BATTERY, charging: false, awake: true, follow_power: true }
    }

    // IDLE only dims the display; treating it as asleep would black out a screen still in use.
    fn is_awake(&self, mode: Option<PowerMode>) -> bool {
        !self.follow_power || !matches!(mode, Some(PowerMode::LowPower | PowerMode::Suspend))
    }

    /// Whether the remote just woke up.
    pub(crate) fn on_power_mode(&mut self, p: &PowerModeChange) -> bool {
        tracing::info!(mode = ?p.mode, "power mode");
        self.awake = self.is_awake(Some(p.mode));
        if let Some(on) = on_charger(p.on_charger()) {
            self.charging = on;
        }
        p.mode == PowerMode::Normal
    }

    pub(crate) fn on_battery(&mut self, b: &BatteryStatus) {
        self.battery = b.capacity.into();
        if let Some(on) = on_charger(b.power_supply) {
            self.charging = on;
        }
    }
}

/// Whether the remote sits on its charger, as reported. Previews of the simulator (always on its charger) can
/// pretend it is on battery.
fn on_charger(reported: Option<bool>) -> Option<bool> {
    if cfg!(feature = "preview") && std::env::var_os("UC_PREVIEW_ON_BATTERY").is_some() {
        return Some(false);
    }
    reported
}

/// The menu's current values. Each is optional: a core that lacks a message just leaves a blank.
pub(crate) async fn load_settings(weak: &slint::Weak<AppWindow>, core: &coreapi::Core) {
    let (display, haptic, power, sound) =
        tokio::join!(core.fetch("get_display_cfg"), core.fetch("get_haptic_cfg"), core.fetch("get_power_saving_cfg"), core.fetch("get_sound_cfg"));
    feedback::set_core_config(
        haptic.as_ref().and_then(|v| v.get("enabled")).and_then(Value::as_bool),
        sound.as_ref().and_then(|v| v.get("enabled")).and_then(Value::as_bool),
        sound.as_ref().and_then(|v| v.get("volume")).and_then(Value::as_u64).map(|v| v.min(100) as u8),
    );
    let secs = |v: &Option<Value>, key: &str| v.as_ref().and_then(|v| v.get(key)).and_then(Value::as_i64).map(duration_label);
    let brightness = display.as_ref().and_then(|v| v.get("brightness")).and_then(Value::as_i64);
    let auto = display.as_ref().and_then(|v| v.get("auto_brightness")).and_then(Value::as_bool);
    let haptics = haptic.as_ref().and_then(|v| v.get("enabled")).and_then(Value::as_bool);
    let (screen_off, sleep) = (secs(&power, "display_off_sec"), secs(&power, "standby_sec"));
    ui(weak, move |app| {
        if let Some(b) = brightness {
            app.global::<Prefs>().set_brightness(b as i32);
        }
        if let Some(a) = auto {
            app.global::<Prefs>().set_auto_brightness(a);
        }
        app.global::<Prefs>().set_haptics(haptics.map_or("", |on| if on { "On" } else { "Off" }).into());
        app.global::<Prefs>().set_screen_off_after(screen_off.unwrap_or_default().into());
        app.global::<Prefs>().set_sleep_after(sleep.unwrap_or_default().into());
    });
}

pub(crate) fn set_display(core: &coreapi::Core, brightness: i32, auto: bool) {
    let core = core.clone();
    tokio::spawn(async move {
        let cfg = json!({ "brightness": brightness, "auto_brightness": auto });
        match core.request("set_display_cfg", cfg).await {
            Ok(r) if r.is_ok() => {}
            other => tracing::warn!("set_display_cfg: {other:?}"),
        }
    });
}
