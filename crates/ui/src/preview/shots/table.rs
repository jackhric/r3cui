//! The stills: one entry per screen, each putting the model and the window in the state to be photographed.

use super::acts::open_picker;
use super::scene::Scene;
use crate::view::screen::Screen;
use crate::NavKey as N;
use crate::{AppWindow, Capture, Control, Fx, JoinState, Kb, PageId, Picker, Sys, Wifi};
use drivers::demo::data::{ANDROID_TV, KODI, PLAYSTATION, START_INPUT, XBOX};
use slint::{ComponentHandle, Model as _};
use std::time::Duration;

pub(super) type Step = (&'static str, fn(&AppWindow, &mut Scene));

/// D-pad presses that type a plausible password, wandering over all four rows.
pub(super) const TYPING: &[N] = &[N::Ok, N::Right, N::Ok, N::Up, N::Ok, N::Left, N::Left, N::Ok, N::Down, N::Down, N::Ok, N::Right, N::Right, N::Ok, N::Up, N::Ok, N::Up, N::Right, N::Ok, N::Ok];

/// The home screen on Kodi with a stream in this audio format, to see its badge.
macro_rules! codec {
    ($name:literal, $format:literal) => {
        ($name, |app, m| {
            app.go_to(PageId::Home, 0);
            m.charging = false;
            m.input = KODI.into();
            m.play();
            m.media_format = $format.into();
        })
    };
}

pub(super) const STEPS: &[Step] = &[
    ("01-receiver", |_, _| {}),
    ("02-inputs", |app, _| app.go_to(PageId::Inputs, app.global::<Control>().get_input_index())),
    ("03-kodi-playing", |app, m| {
        app.go_to(PageId::Home, 0);
        m.input = KODI.into();
    }),
    ("03b-audio", |app, _| open_picker(app, Picker::Audio)),
    ("03c-subtitles", |app, _| open_picker(app, Picker::Subtitles)),
    ("03d-paused", |app, m| {
        app.go_to(PageId::Home, 0);
        m.paused = true;
    }),
    ("04-kodi-browsing", |_, m| {
        m.paused = false;
        m.title.clear();
    }),
    ("05-volume", |app, m| {
        m.play();
        m.volume = 64;
        // Blur Now playing, not the stage it replaces.
        after_swap(app, |app| {
            crate::view::stage::capture(app, Capture::World);
            app.global::<Control>().set_volume_overlay(true);
        });
    }),
    ("06-muted", |_, m| m.muted = true),
    ("07-menu", |app, m| {
        app.global::<Control>().set_volume_overlay(false);
        m.muted = false;
        m.volume = 62;
        app.go_to(PageId::Menu, 1);
    }),
    ("07b-menu-playstation", |_, m| m.input = PLAYSTATION.into()),
    ("07c-menu-xbox", |_, m| m.input = XBOX.into()),
    ("07d-menu-kodi", |_, m| m.input = KODI.into()),
    ("08-menu-display", |app, _| app.go_to(PageId::Display, 0)),
    ("09-menu-power", |app, _| app.go_to(PageId::Power, 0)),
    ("10-playstation", |app, m| {
        app.go_to(PageId::Home, 0);
        m.input = START_INPUT.into();
    }),
    ("11-xbox", |_, m| m.input = XBOX.into()),
    ("12-android-tv", |_, m| m.input = ANDROID_TV.into()),
    ("13-receiver-off", |_, m| {
        m.input = KODI.into();
        m.power_on = false;
    }),
    ("14-wifi", |app, m| {
        m.power_on = true;
        app.go_to(PageId::Wifi, 1);
    }),
    ("15-keyboard", |app, _| {
        app.global::<Wifi>().set_target(app.global::<Wifi>().get_rows().row_data(1).unwrap_or_default());
        app.global::<Kb>().invoke_reset();
        app.go_to(PageId::Keyboard, 0);
        for k in TYPING {
            app.global::<Kb>().invoke_nav(*k);
        }
    }),
    ("16-joining", |app, _| app.global::<Wifi>().set_join_state(JoinState::Joining)),
    ("17-join-failed", |app, _| {
        app.global::<Wifi>().set_join_state(JoinState::Failed);
        app.go_to(PageId::JoinFailed, 0);
    }),
    ("18-status", |app, _| {
        app.global::<Wifi>().set_join_state(JoinState::Idle);
        app.go_to(PageId::Status, 0);
    }),
    ("19-status-ha-down", |_, m| m.links_down = true),
    ("20-home-warning", |app, _| app.go_to(PageId::Home, 0)),
    ("21-charging", |_, m| {
        m.links_down = false;
        m.charging = true;
    }),
    ("22-charging-home", |app, _| app.global::<Sys>().set_charge_card(false)),
    ("23-charging-kodi", |_, m| {
        m.input = KODI.into();
        m.play();
    }),
    codec!("30-codec-00", "TrueHD Atmos 7.1"),
    codec!("30-codec-01", "DD+ Atmos 5.1"),
    codec!("30-codec-02", "DTS:X 7.1"),
    codec!("30-codec-03", "DTS-HD MA 7.1"),
    codec!("30-codec-04", "DTS-HD HRA 5.1"),
    codec!("30-codec-05", "DTS 5.1"),
    codec!("30-codec-06", "TrueHD 7.1"),
    codec!("30-codec-07", "DD+ 5.1"),
    codec!("30-codec-08", "Dolby Digital 5.1"),
    codec!("30-codec-09", "FLAC 2.0"),
    codec!("30-codec-10", "Alac 2 channels"),
    codec!("30-codec-11", "Opus 5.1"),
    codec!("30-codec-12", "MP3 stereo"),
    codec!("30-codec-13", "AAC-LC stereo"),
    codec!("30-codec-14", "Pcm_S24Le 2 channels"),
];

/// Run `f` once the stage swap under way has finished.
pub(super) fn after_swap(app: &AppWindow, f: impl FnOnce(&AppWindow) + 'static) {
    let weak = app.as_weak();
    slint::Timer::single_shot(Duration::from_millis(300), move || {
        let Some(app) = weak.upgrade() else { return };
        if app.global::<Fx>().get_swapping() {
            after_swap(&app, f);
        } else {
            f(&app);
        }
    });
}
