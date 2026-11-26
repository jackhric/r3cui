use super::acts::{button, focus_step, kodi_hold, key, power_key, press, swipe, tap_at, tap_strip, ui, Act, Tx};
use super::shelf::shelf;
use crate::input::Button;
use crate::view::screen::Screen;
use crate::{AppWindow, Control, Lib, LibMode, Nav, PageId, Prefs, UiAction};
use drivers::demo::data::KODI;
use slint::ComponentHandle;
use std::rc::Rc;

/// Short clips for the website's hero, 6 to 9 seconds each after a settling lead-in (`SITE_LEAD`
/// ms, trimmed when the clip is encoded). Each starts from a known input, `UC_SITE_SOURCE` (Kodi by
/// default), so the same clip can be recorded in each input's colours. `site-switch` then moves
/// `UC_SITE_STEPS` rows down the input list and picks that input.
pub(super) const SITE_LEAD: u64 = 2500;

pub(super) fn site_script(name: &str) -> Vec<(u64, Act)> {
    let start = std::env::var("UC_SITE_SOURCE").unwrap_or_else(|_| KODI.into());
    let first: Act = Box::new(move |_, tx| {
        let _ = tx.send(UiAction::SelectInput(start.clone()));
    });
    let mut s: Vec<(u64, Act)> = vec![(300, first), (SITE_LEAD - 300, ui(|_| {}))];
    s.extend(match name {
        "site-inputs" => vec![
            (900, ui(|a| a.go_to(PageId::Inputs, a.global::<Control>().get_input_index()))),
            (700, ui(|a| a.global::<Nav>().invoke_move(1))),
            (700, ui(|a| a.global::<Nav>().invoke_activate(a.global::<Nav>().get_focus()))),
            (2300, ui(|a| a.go_to(PageId::Inputs, a.global::<Control>().get_input_index()))),
            (700, ui(|a| a.global::<Nav>().invoke_move(1))),
            (700, ui(|a| a.global::<Nav>().invoke_activate(a.global::<Nav>().get_focus()))),
            (2400, ui(|_| {})),
        ],
        "site-switch" => {
            let steps: i32 = std::env::var("UC_SITE_STEPS").ok().and_then(|n| n.parse().ok()).unwrap_or(1);
            let mut v: Vec<(u64, Act)> = vec![(1400, ui(|a| a.go_to(PageId::Inputs, a.global::<Control>().get_input_index())))];
            for _ in 0..steps.abs() {
                v.push((550, Box::new(move |a: &AppWindow, _: &Tx| a.global::<Nav>().invoke_move(steps.signum()))));
            }
            v.push((700, ui(|a| a.global::<Nav>().invoke_activate(a.global::<Nav>().get_focus()))));
            v.push((3600, ui(|_| {})));
            v
        }
        // The wall scrolling: down three rows, one across, and back up, each a d-pad press sent to Kodi.
        "site-wall" => {
            const UP: &str = "\u{F700}";
            const DOWN: &str = "\u{F701}";
            const RIGHT: &str = "\u{F703}";
            vec![
                (3000, key(DOWN)),
                (1300, key(DOWN)),
                (1300, key(DOWN)),
                (1300, key(RIGHT)),
                (1300, key(UP)),
                (1300, key(UP)),
                (1300, key(UP)),
                (1600, ui(|_| {})),
            ]
        }
        // The remote drives Kodi itself: two d-pad rights along one row of the library to 2001, OK, OK again on
        // Kodi's resume prompt, a few seconds on Now Playing, then the subtitle picker, taken slowly, to English.
        // Once it is on, Kodi is paused and its read-ahead reloaded so the subtitles show (see
        // `kodi::reload`); the pause, marked in the output, is cut from the video.
        "site-kodi-remote" => {
            const RIGHT: &str = "\u{F703}";
            vec![
                (2500, key(RIGHT)),
                (1300, key(RIGHT)),
                (1400, button(Button::DpadMiddle)),
                (1500, button(Button::DpadMiddle)),
                (4600, tap_at(132.0, 746.0)),
                (1800, ui(|a| a.global::<Nav>().invoke_move(1))),
                (1700, ui(|a| a.global::<Nav>().invoke_activate(a.global::<Nav>().get_focus()))),
                (300, kodi_hold(true)),
                (3000, kodi_hold(false)),
                // Kodi replays the seconds it went back over, which the edit cuts too.
                (6500, ui(|_| {})),
            ]
        }
        // Kodi's library on the wall while a helper steps Kodi across to a film and plays it (about 11 s), then the
        // audio and subtitle pickers.
        "site-kodi" => vec![
            (11000, ui(|_| {})),
            (900, tap_at(348.0, 746.0)),
            (1500, ui(|a| a.global::<Nav>().invoke_move(1))),
            (900, press(Button::DpadMiddle.key_text())),
            (1400, tap_at(132.0, 746.0)),
            (1400, ui(|a| a.global::<Nav>().invoke_move(1))),
            (900, ui(|a| a.global::<Nav>().invoke_activate(a.global::<Nav>().get_focus()))),
            (1600, ui(|_| {})),
        ],
        // The audio picker, then the subtitle picker, held long enough to read their track names.
        "site-tracks" => vec![
            (900, tap_at(348.0, 746.0)),
            (1600, ui(|a| a.global::<Nav>().invoke_move(1))),
            (900, press(Button::DpadMiddle.key_text())),
            (1500, tap_at(132.0, 746.0)),
            (1500, ui(|a| a.global::<Nav>().invoke_move(1))),
            (1000, ui(|a| a.global::<Nav>().invoke_activate(a.global::<Nav>().get_focus()))),
            (1800, ui(|_| {})),
        ],
        "site-playing" => vec![
            (1200, tap_at(240.0, 584.0)),
            (1300, tap_at(240.0, 584.0)),
            (1300, tap_at(348.0, 746.0)),
            (1100, ui(|a| a.global::<Nav>().invoke_move(1))),
            (600, press(Button::DpadMiddle.key_text())),
            (1300, tap_at(132.0, 746.0)),
            (1100, ui(|a| a.global::<Nav>().invoke_move(1))),
            (600, ui(|a| a.global::<Nav>().invoke_activate(a.global::<Nav>().get_focus()))),
            (1500, ui(|_| {})),
        ],
        // Keys tick the level up and down, the slider sweeps it, the overlay fades back to the film, then a
        // double tap mutes and unmutes.
        "site-volume" => vec![
            (900, button(Button::VolumeUp)),
            (260, button(Button::VolumeUp)),
            (260, button(Button::VolumeUp)),
            (260, button(Button::VolumeUp)),
            (700, button(Button::VolumeDown)),
            (260, button(Button::VolumeDown)),
            (900, swipe((0..30).map(|i| 0.35 + i as f32 * 0.012).collect())),
            (2600, tap_strip()),
            (180, tap_strip()),
            (1500, tap_strip()),
            (180, tap_strip()),
            (2000, ui(|_| {})),
        ],
        "site-power" => vec![
            (1000, power_key(true)),
            (1500, power_key(false)),
            (2200, power_key(true)),
            (1500, power_key(false)),
            (2200, ui(|_| {})),
        ],
        "site-menu" => vec![
            (900, ui(|a| a.go_to(PageId::Menu, 0))),
            (700, ui(|a| a.global::<Nav>().invoke_move(1))),
            (600, ui(|a| a.global::<Nav>().invoke_activate(1))),
            (800, ui(|a| a.global::<Prefs>().set_brightness(70))),
            (500, ui(|a| a.global::<Prefs>().set_brightness(45))),
            (600, ui(|a| a.global::<Prefs>().set_brightness(90))),
            (900, ui(|a| a.global::<Nav>().invoke_back())),
            (600, ui(|a| a.global::<Nav>().invoke_move(1))),
            (350, ui(|a| a.global::<Nav>().invoke_move(1))),
            (600, ui(|a| a.global::<Nav>().invoke_activate(3))),
            (1300, ui(|a| a.global::<Nav>().invoke_back())),
            (800, ui(|a| a.global::<Nav>().invoke_back())),
            (1200, ui(|_| {})),
        ],
        "site-library" => {
            let movies = Rc::new(shelf("movies"));
            let focus = movies.focus();
            let load: Act = Box::new(move |app, _| {
                movies.list(app, "MOVIES");
                app.global::<Lib>().set_focus((focus - 3).max(0));
            });
                    vec![
                (100, load),
                (600, button(Button::Stop)),
                (1500, ui(|a| crate::view::stage::library_stage(a, LibMode::Wall))),
                (1600, focus_step(1)),
                (700, focus_step(1)),
                (700, focus_step(1)),
                (2600, ui(|_| {})),
            ]
        }
        _ => return Vec::new(),
    });
    s
}
