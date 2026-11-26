//! Preview captures with demo data. `--shots <dir>` saves a PNG per screen; `--record <file.mp4>`
//! plays a scripted session and pipes frames to ffmpeg. With `--frame <photo>` both show the UI
//! inside the remote with its keys lit.

mod acts;
mod record;
mod scene;
mod scripts;
mod shelf;
mod site;
mod table;

pub use record::record;

use crate::app::demo::apply_demo_settings;
use crate::hardware::backlight;
use crate::preview::device::Device;
use crate::{view::apply, AppWindow, Fx, Sys};
use scene::Scene;
use slint::ComponentHandle;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};
use table::STEPS;

/// What the keys show for what the window is showing.
pub(super) fn keys(app: &AppWindow) -> backlight::Keys {
    let (state, accent) = backlight::observe(app);
    backlight::keys_for(state, accent)
}

pub fn run(app: AppWindow, dir: PathBuf, mut frame: Option<Device>) -> anyhow::Result<()> {
    std::fs::create_dir_all(&dir)?;
    apply_demo_settings(&app);
    app.global::<Sys>().set_connected(true);
    let scene = Rc::new(RefCell::new(Scene::new()));
    apply(&app, &scene.borrow().shown());
    let _lava = crate::view::idle::animate_lava(&app);

    let timer = slint::Timer::default();
    let weak = app.as_weak();
    let mut next = 0usize;
    let mut since = Instant::now();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(100), move || {
        let Some(app) = weak.upgrade() else { return };
        // A stage swap can outlast its nominal 620 ms while the blur pictures are built, so wait
        // for it to end and then for the page slide and fades to settle.
        if app.global::<Fx>().get_swapping() {
            since = Instant::now();
        }
        if since.elapsed() < Duration::from_millis(1500) {
            return;
        }
        since = Instant::now();
        if next > 0 {
            let path = dir.join(format!("{}.png", STEPS[next - 1].0));
            let saved = app.window().take_snapshot().map_err(anyhow::Error::from).and_then(|buf| {
                match &mut frame {
                    Some(device) => device
                        .compose(buf.as_bytes(), (buf.width(), buf.height()), keys(&app), f32::MAX)
                        .save(&path)?,
                    None => image::save_buffer(&path, buf.as_bytes(), buf.width(), buf.height(), image::ColorType::Rgba8)?,
                }
                Ok(())
            });
            println!("{} {saved:?}", path.display());
        }
        match STEPS.get(next) {
            Some((_, step)) => {
                let mut scene = scene.borrow_mut();
                step(&app, &mut scene);
                apply(&app, &scene.shown());
                next += 1;
            }
            None => {
                let _ = slint::quit_event_loop();
            }
        }
    });

    app.run()?;
    Ok(())
}

/// `--snap <png>`: one picture of whatever the app is showing a few seconds after launch, then quit.
/// Works in live mode, so it shows real data from the core.
pub fn snap(app: &AppWindow, path: PathBuf, mut frame: Option<Device>) -> slint::Timer {
    let weak = app.as_weak();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::SingleShot, Duration::from_secs(5), move || {
        let Some(app) = weak.upgrade() else { return };
        let saved = app.window().take_snapshot().map_err(anyhow::Error::from).and_then(|buf| {
            match &mut frame {
                Some(device) => device
                    .compose(buf.as_bytes(), (buf.width(), buf.height()), keys(&app), f32::MAX)
                    .save(&path)?,
                None => image::save_buffer(&path, buf.as_bytes(), buf.width(), buf.height(), image::ColorType::Rgba8)?,
            }
            Ok(())
        });
        println!("{} {saved:?}", path.display());
        let _ = slint::quit_event_loop();
    });
    timer
}
