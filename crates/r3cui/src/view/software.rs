//! The System pages' values into the window.

use crate::app::software::View;
use crate::view::{qr, ui};
use crate::Software as Shown;
use slint::ComponentHandle;
use std::cell::RefCell;

/// How large the QR codes are drawn, at most, in pixels a side.
const RELEASE_QR: u32 = 168;
const WEB_QR: u32 = 176;

thread_local! {
    /// The text of the codes now in the window, so each is drawn once.
    static DRAWN: RefCell<(String, String)> = const { RefCell::new((String::new(), String::new())) };
}

fn code(text: &str, target: u32) -> slint::Image {
    if text.is_empty() {
        return slint::Image::default();
    }
    qr::image(text, target).unwrap_or_default()
}

pub(crate) fn show(weak: &slint::Weak<crate::AppWindow>, v: View) {
    ui(weak, move |app| {
        let s = app.global::<Shown>();
        s.set_r3_version(v.r3_version.into());
        s.set_r3_commit(v.r3_build.commit.into());
        s.set_r3_features(v.r3_build.features.into());
        s.set_r3_repo(v.r3_repo.into());
        s.set_r3_state(v.r3_state);
        s.set_r3_status(v.r3_status.into());
        s.set_r3_new_version(v.r3_new_version.into());
        s.set_r3_release_line(v.r3_release_line.into());
        s.set_r3_phase(v.r3_phase);
        s.set_r3_percent(v.r3_percent.into());
        s.set_r3_error(v.r3_error.into());
        s.set_uninstall_state(v.uninstall_state);
        s.set_uninstall_error(v.uninstall_error.into());
        s.set_fw_installed(v.fw_installed.into());
        s.set_fw_state(v.fw_state);
        s.set_fw_status(v.fw_status.into());
        s.set_fw_new_version(v.fw_new_version.into());
        s.set_fw_release_line(v.fw_release_line.into());
        s.set_fw_notes(v.fw_notes.into());
        s.set_fw_battery_line(v.fw_battery_line.into());
        s.set_fw_battery_ok(v.fw_battery_ok);
        s.set_fw_error(v.fw_error.into());
        s.set_fw_phase(v.fw_phase);
        s.set_fw_percent(v.fw_percent.into());
        s.set_fw_step_line(v.fw_step_line.into());
        s.set_web_on(v.web_on);
        s.set_web_pin(slint::ModelRc::new(slint::VecModel::from(v.web_pin.chars().map(|c| slint::SharedString::from(c.to_string())).collect::<Vec<_>>())));
        s.set_web_address(v.web_address.into());
        DRAWN.with(|drawn| {
            let mut drawn = drawn.borrow_mut();
            if drawn.0 != v.r3_qr_text {
                s.set_r3_qr(code(&v.r3_qr_text, RELEASE_QR));
                drawn.0 = v.r3_qr_text;
            }
            if drawn.1 != v.web_qr_text {
                s.set_web_qr(code(&v.web_qr_text, WEB_QR));
                drawn.1 = v.web_qr_text;
            }
        });
    });
}
