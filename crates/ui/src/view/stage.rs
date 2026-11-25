//! The stage behind the controls (now playing, inputs, the poster wall, off) and the blurred crossfade from one
//! to the next.

use crate::AppWindow;
use crate::app::model::Model;
use crate::art::pixels;

use slint::ComponentHandle;
use std::cell::RefCell;
use std::time::Duration;

/// The stage as it looks right now, alone and unreceded.
pub(crate) fn snapshot_stage(app: &AppWindow) -> Option<image::RgbaImage> {
    app.set_capturing(1);
    let shot = app.window().take_snapshot();
    app.set_capturing(0);
    let shot = shot.map_err(|e| tracing::warn!("stage snapshot: {e}")).ok()?;
    image::RgbaImage::from_raw(shot.width(), shot.height(), shot.as_bytes().to_vec())
}
pub(crate) fn soften(img: &image::RgbaImage, sigma: f32) -> slint::Image {
    let small = image::imageops::resize(img, 60, 100, image::imageops::FilterType::Triangle);
    slint::Image::from_rgba8(pixels(image::imageops::blur(&small, sigma)))
}
/// What decides which main stage is showing.
#[derive(Clone, PartialEq)]
pub(crate) struct StageKey {
    receiver_on: bool,
    playing: bool,
    source: String,
    /// The poster wall, standing in for the idle stage.
    library: bool,
}
thread_local!(static STAGE: RefCell<Option<StageKey>> = const { RefCell::new(None) });
/// Apply `change`, and if it moved the window to a different main stage while the stage is in
/// view, blur through from the old one to the new.
pub(crate) fn stage_change(app: &AppWindow, m: &Model, change: impl FnOnce()) {
    let playing = m.on_kodi() && !m.title.is_empty();
    let key = StageKey { receiver_on: m.receiver_on, playing, source: m.source.clone(), library: app.get_lib_mode() > 0 && !playing };
    swap_stage(app, key, change);
}
/// Show or hide the poster wall, blurring across like any other change of stage.
pub(crate) fn library_stage(app: &AppWindow, mode: i32) {
    if app.get_lib_mode() == mode {
        return;
    }
    let Some(mut key) = STAGE.with(|s| s.borrow().clone()) else {
        app.set_lib_mode(mode);
        return;
    };
    key.library = mode > 0 && !key.playing;
    swap_stage(app, key, || {
        app.set_lib_mode(mode);
        if mode > 0 {
            app.invoke_hold_library();
        }
    });
    if mode > 0 {
        // Under way as the old stage's blur clears, rather than hidden beneath it.
        reveal_library(app, Duration::from_millis(250));
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
    let visible = app.get_page() == 0 && app.get_lit() && app.get_capturing() == 0;
    if before.is_none() || before.as_ref() == Some(&key) || !visible {
        change();
        return;
    }
    let old = snapshot_stage(app);
    change();
    let new = snapshot_stage(app);
    let (Some(old), Some(new)) = (old, new) else { return };
    let plain = before.is_some_and(|b| b.receiver_on == key.receiver_on && b.source == key.source);
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
pub(crate) fn capture(app: &AppWindow, which: i32) {
    app.set_capturing(which + 1);
    let shot = app.window().take_snapshot();
    app.set_capturing(0);
    let shot = match shot {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("snapshot for blur: {e}");
            return;
        }
    };
    let Some(full) = image::RgbaImage::from_raw(shot.width(), shot.height(), shot.as_bytes().to_vec()) else { return };
    let small = image::imageops::resize(&full, 60, 100, image::imageops::FilterType::Triangle);
    let soft = image::imageops::blur(&small, 1.6);
    let img = slint::Image::from_rgba8(pixels(soft));
    if which == 0 {
        app.set_stage_blur(img);
    } else {
        app.set_world_blur(img);
    }
}
