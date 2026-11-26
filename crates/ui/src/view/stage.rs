//! The stage behind the controls (now playing, inputs, the poster wall, off) and the blurred crossfade from one
//! to the next.

use crate::{AppWindow, Capture, LibMode};
use crate::app::model::Model;
use crate::art::pixels;
use crate::view::screen::{Page, Screen};

use slint::ComponentHandle;
use std::cell::RefCell;
use std::time::Duration;

/// How long into the old stage's blur-out the library wall starts to come in.
const LIBRARY_REVEAL_LEAD: Duration = Duration::from_millis(250);

/// The stage as it looks right now, alone and unreceded.
/// The window as one blur's picture: the stage alone, or the stage with the world over it.
fn snapshot(app: &AppWindow, what: Capture) -> Option<image::RgbaImage> {
    app.set_capturing(what);
    let shot = app.window().take_snapshot();
    app.set_capturing(Capture::None);
    let shot = shot.map_err(|e| tracing::warn!("snapshot: {e}")).ok()?;
    image::RgbaImage::from_raw(shot.width(), shot.height(), shot.as_bytes().to_vec())
}
pub(crate) fn snapshot_stage(app: &AppWindow) -> Option<image::RgbaImage> {
    snapshot(app, Capture::Stage)
}
pub(crate) fn soften(img: &image::RgbaImage, sigma: f32) -> slint::Image {
    let small = image::imageops::resize(img, 60, 100, image::imageops::FilterType::Triangle);
    slint::Image::from_rgba8(pixels(image::imageops::blur(&small, sigma)))
}
/// What decides which main stage is showing.
#[derive(Clone, PartialEq)]
pub(crate) struct StageKey {
    power_on: bool,
    playing: bool,
    input: String,
    /// The poster wall, standing in for the idle stage.
    library: bool,
}
thread_local!(static STAGE: RefCell<Option<StageKey>> = const { RefCell::new(None) });
/// Apply `change`, and if it moved the window to a different main stage while the stage is in
/// view, blur through from the old one to the new.
pub(crate) fn stage_change(app: &AppWindow, m: &Model, change: impl FnOnce()) {
    let playing = m.staged && !m.title.is_empty();
    let key = StageKey { power_on: m.power_on, playing, input: m.input.clone(), library: app.get_lib_mode() != LibMode::Off && !playing };
    swap_stage(app, key, change);
}
/// Show or hide the poster wall, blurring across like any other change of stage.
pub(crate) fn library_stage(app: &AppWindow, mode: LibMode) {
    if app.get_lib_mode() == mode {
        return;
    }
    let Some(mut key) = STAGE.with(|s| s.borrow().clone()) else {
        app.set_lib_mode(mode);
        return;
    };
    key.library = mode != LibMode::Off && !key.playing;
    swap_stage(app, key, || {
        app.set_lib_mode(mode);
        if mode != LibMode::Off {
            app.invoke_hold_library();
        }
    });
    if mode != LibMode::Off {
        // Under way as the old stage's blur clears, rather than hidden beneath it.
        reveal_library(app, LIBRARY_REVEAL_LEAD);
    }
}
/// Play the wall's entrance from the start after `delay` (at the soonest, the next frame).
pub(crate) fn reveal_library(app: &AppWindow, delay: Duration) {
    app.invoke_hold_library();
    let weak = app.as_weak();
    slint::Timer::single_shot(delay, move || {
        if let Some(app) = weak.upgrade() {
            app.invoke_run_library();
        }
    });
}
pub(crate) fn swap_stage(app: &AppWindow, key: StageKey, change: impl FnOnce()) {
    let before = STAGE.with(|s| s.replace(Some(key.clone())));
    let visible = app.on_page(Page::Home) && app.get_lit() && app.get_capturing() == Capture::None;
    if before.is_none() || before.as_ref() == Some(&key) || !visible {
        change();
        return;
    }
    let old = snapshot_stage(app);
    change();
    let new = snapshot_stage(app);
    let (Some(old), Some(new)) = (old, new) else { return };
    let plain = before.is_some_and(|b| b.power_on == key.power_on && b.input == key.input);
    app.set_swap_plain(plain);
    app.set_swap_out_blur(soften(&old, 2.2));
    app.set_swap_in_blur(soften(&new, 2.2));
    app.set_swap_out(slint::Image::from_rgba8(pixels(old)));
    app.invoke_hold_swap();
    let weak = app.as_weak();
    slint::Timer::single_shot(Duration::ZERO, move || {
        if let Some(app) = weak.upgrade() {
            app.invoke_run_swap();
        }
    });
}
/// A soft, blurred picture of the screen for a layer that is about to recede: 0 the stage alone,
/// 1 everything under the overlays.
pub(crate) fn capture(app: &AppWindow, what: Capture) {
    let Some(full) = snapshot(app, what) else { return };
    let img = soften(&full, 1.6);
    match what {
        Capture::Stage => app.set_stage_blur(img),
        Capture::World => app.set_world_blur(img),
        Capture::None => {}
    }
}
