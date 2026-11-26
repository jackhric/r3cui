//! The one way input reaches the window: the remote's keys and touchscreen, and remote control, so a virtual press
//! or touch does exactly what a real one does (wakes the screen, keeps frames coming, haptics and all).

use crate::AppWindow;
use slint::platform::WindowEvent;
use slint::ComponentHandle;

pub(crate) fn deliver(weak: &slint::Weak<AppWindow>, event: WindowEvent) {
    crate::view::idle::poke();
    let weak = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(app) = weak.upgrade() {
            #[cfg(feature = "device")]
            crate::hardware::frames::kick();
            app.window().dispatch_event(event);
        }
    });
}
