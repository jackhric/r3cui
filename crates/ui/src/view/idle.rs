//! Time since the last key or touch, resting on the charger, and the menu's lava animation.

use crate::view::screen::{Page, Screen};
use crate::{AppWindow, Theme};
use crate::app::settings;
use crate::view::lava;

use slint::ComponentHandle;
use std::time::{Duration, Instant};

/// On the charger, the screen and keys go dark after this long without a touch or key.
pub(crate) const CHARGER_REST: Duration = Duration::from_secs(10);
pub(crate) static LAST_INPUT: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
/// A key or touch: the charger's countdown to dark starts over.
pub(crate) fn poke() {
    *LAST_INPUT.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
}
pub(crate) fn since_input() -> Duration {
    LAST_INPUT.lock().unwrap_or_else(|e| e.into_inner()).map_or(Duration::MAX, |t| t.elapsed())
}
/// On the charger nobody is holding the remote, so the screen and keys go dark soon. The core's
/// own display timeout still applies on top of this.
pub(crate) fn rest_on_charger(app: &AppWindow) -> slint::Timer {
    let weak = app.as_weak();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(500), move || {
        let Some(app) = weak.upgrade() else { return };
        // Typing a Wi-Fi password is slow; never black out the keyboard.
        let busy = app.on_page(Page::Password) || app.get_power_overlay();
        if app.get_charging() && app.get_lit() && !busy && since_input() >= CHARGER_REST {
            app.set_resting(true);
            app.set_charge_card(false);
        }
    });
    timer
}
/// Redraws the idle home screen's background while it is on screen, at a calm 20 fps.
pub(crate) fn animate_lava(app: &AppWindow) -> slint::Timer {
    let weak = app.as_weak();
    let start = Instant::now();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(50), move || {
        let Some(app) = weak.upgrade() else { return };
        if !app.get_lava_on() {
            return;
        }
        let a = app.global::<Theme>().get_accent();
        let colours = lava::inverted(lava::palette((a.red(), a.green(), a.blue()), settings::get().lava.unwrap_or(1.0)));
        app.set_lava(slint::Image::from_rgb8(lava::render(start.elapsed().as_secs_f32() * 0.6, colours)));
    });
    timer
}
