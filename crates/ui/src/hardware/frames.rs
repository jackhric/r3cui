//! Keeps animations at the display's refresh rate on the remote.
//!
//! Measured on the device: drawing continuously gives ~58 fps, but left to itself the KMS backend
//! schedules animation frames lazily and presents only every other refresh (~30 fps). So while
//! something is moving, a redraw is requested every few milliseconds and the backend's own vsync
//! wait paces it; once things settle the timer stops, so a still screen costs nothing.

use crate::AppWindow;
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Long enough to cover the slowest transition that a single event starts.
const SETTLE: Duration = Duration::from_millis(1500);
const TICK: Duration = Duration::from_millis(5);

struct Pump {
    timer: Rc<slint::Timer>,
    until: Rc<Cell<Instant>>,
    app: slint::Weak<AppWindow>,
}

thread_local! {
    static PUMP: RefCell<Option<Pump>> = const { RefCell::new(None) };
}

/// UI thread, once.
pub fn install(app: &AppWindow) {
    let pump = Pump { timer: Rc::new(slint::Timer::default()), until: Rc::new(Cell::new(Instant::now())), app: app.as_weak() };
    PUMP.with(|p| *p.borrow_mut() = Some(pump));
}

/// Something is about to move: keep frames coming for a while. UI thread only.
pub fn kick() {
    PUMP.with(|p| {
        let p = p.borrow();
        let Some(pump) = p.as_ref() else { return };
        pump.until.set(Instant::now() + SETTLE);
        if pump.timer.running() {
            return;
        }
        let (timer, until, app) = (pump.timer.clone(), pump.until.clone(), pump.app.clone());
        pump.timer.start(slint::TimerMode::Repeated, TICK, move || {
            let Some(app) = app.upgrade() else { return };
            // The power-hold fill and the joining spinner run for as long as they are on screen.
            if Instant::now() < until.get() || app.get_power_overlay() || app.get_join_state() == 1 {
                app.window().request_redraw();
            } else {
                timer.stop();
            }
        });
    });
}
