//! The scripted recordings (`--record <file> --script <name>`): (pause before the step in ms, what happens).

use super::acts::{button, focus_step, key, open_picker, power_key, press, select_input, swipe, tap_at, tap_strip, ui, Act, Tx};
use super::shelf::{shelf, Shelf};
use super::site::site_script;
use super::table::TYPING;
use crate::input::{keys, Button};
use crate::view::screen::Screen;
use crate::{AppWindow, Control, Kb, Lib, LibMode, Nav, PageId, Picker, Prefs, UiAction, WallStyle};
use drivers::demo::data::{ANDROID_TV, KODI, PLAYSTATION, XBOX};
use slint::ComponentHandle;
use std::rc::Rc;
use std::time::Duration;

/// Sleep and wake, on the Kodi screen and then on the receiver screen.
pub(super) fn power_script() -> Vec<(u64, Act)> {
    vec![
        (300, select_input(KODI)),
        (2200, key("z")),
        (2400, key("z")),
        (2800, key("z")),
        (2200, select_input(PLAYSTATION)),
        (300, key("z")),
        (2800, key("z")),
        (2400, ui(|_| {})),
    ]
}

/// The physical power key, through real key events: a short tap (nothing happens), then a hold
/// (receiver off), from the menu to show it works anywhere, then a hold to turn it back on.
pub(super) fn hold_script() -> Vec<(u64, Act)> {
    vec![
        (1500, power_key(true)),
        (450, power_key(false)),
        (1800, ui(|a| a.go_to(PageId::Menu, 0))),
        (1300, power_key(true)),
        (1500, power_key(false)),
        (1400, ui(|a| a.global::<Nav>().invoke_back())),
        (2200, power_key(true)),
        (1500, power_key(false)),
        (2200, ui(|_| {})),
    ]
}

/// Menu → Wi-Fi → pick a network → type its password with the d-pad → a failed join, a retry
/// that works, then the status page with Home Assistant down and the warning on the home screen.
pub(super) fn wifi_script() -> Vec<(u64, Act)> {
    let mut steps: Vec<(u64, Act)> = vec![
        (1200, ui(|a| a.go_to(PageId::Menu, 0))),
        (800, ui(|a| a.global::<Nav>().invoke_activate(0))),
        (2400, ui(|a| a.global::<Nav>().invoke_move(1))),
        (700, ui(|a| a.global::<Nav>().invoke_activate(1))),
        (900, ui(|_| {})),
    ];
    for k in TYPING {
        steps.push((260, Box::new(move |a: &AppWindow, _: &Tx| a.global::<Kb>().invoke_nav(*k))));
    }
    steps.extend([
        (700, ui(|a| a.global::<Kb>().invoke_tap(30))),
        (3400, ui(|a| a.global::<Nav>().invoke_activate(0))),
        (1100, ui(|a| a.global::<Kb>().invoke_tap(30))),
        (3600, ui(|a| a.global::<Nav>().invoke_back())),
        (700, ui(|a| a.global::<Nav>().invoke_activate(4))),
        (1800, key("x")),
        (2200, ui(|a| a.global::<Nav>().invoke_back())),
        (600, ui(|a| a.global::<Nav>().invoke_back())),
        (2600, ui(|_| {})),
    ]);
    steps
}

/// The home key through real key events, on the Kodi screen: a tap (Kodi's own home, nothing
/// changes here), then a hold that opens the input list, then a tap that returns home.
pub(super) fn home_script() -> Vec<(u64, Act)> {
    use slint::platform::WindowEvent;
    let down: fn(&AppWindow) = |a| a.window().dispatch_event(WindowEvent::KeyPressed { text: keys::HOME.into() });
    let up: fn(&AppWindow) = |a| a.window().dispatch_event(WindowEvent::KeyReleased { text: keys::HOME.into() });
    let kodi = select_input(KODI);
    vec![(300, kodi), (1800, ui(down)), (120, ui(up)), (1500, ui(down)), (900, ui(up)), (2200, ui(down)), (120, ui(up)), (1800, ui(|_| {}))]
}

/// Kodi playing: skip back and forward, pause, the slider for volume and a double tap to mute,
/// then the audio and subtitle pickers.
pub(super) fn media_script() -> Vec<(u64, Act)> {
    let kodi = select_input(KODI);
    vec![
        (300, kodi),
        (1800, tap_at(116.0, 580.0)),
        (700, tap_at(116.0, 580.0)),
        (900, tap_at(364.0, 580.0)),
        (1000, tap_at(240.0, 584.0)),
        (1400, tap_at(240.0, 584.0)),
        // Slide right for louder, then back down a little.
        (1200, swipe((0..24).map(|i| 0.3 + i as f32 * 0.012).collect())),
        (1400, swipe((0..14).map(|i| 0.6 - i as f32 * 0.01).collect())),
        // Double tap: mute, and again to unmute.
        (1600, tap_strip()),
        (180, tap_strip()),
        (1800, tap_strip()),
        (180, tap_strip()),
        (2000, tap_at(348.0, 746.0)),
        (1300, ui(|a| a.global::<Nav>().invoke_move(1))),
        (500, press(Button::DpadMiddle.key_text())),
        (1500, tap_at(132.0, 746.0)),
        (1300, ui(|a| a.global::<Nav>().invoke_move(1))),
        (500, ui(|a| a.global::<Nav>().invoke_activate(a.global::<Nav>().get_focus()))),
        (1800, ui(|_| {})),
    ]
}

/// The three layers: a track picker over the playing film, the volume overlay over that, the
/// menu over the stage, and the power hold over everything.
pub(super) fn layers_script() -> Vec<(u64, Act)> {
    let kodi = select_input(KODI);
    vec![
        (300, kodi),
        (1500, tap_at(132.0, 746.0)),
        (1600, ui(|a| a.global::<Nav>().invoke_move(1))),
        (500, button(Button::VolumeUp)),
        (300, button(Button::VolumeUp)),
        (2200, press(Button::DpadMiddle.key_text())),
        (1600, tap_at(348.0, 746.0)),
        (1800, press(Button::Back.key_text())),
        (1400, tap_at(436.0, 32.0)),
        (2600, ui(|a| a.global::<Nav>().invoke_back())),
        (1500, power_key(true)),
        (1500, power_key(false)),
        (2400, ui(|_| {})),
    ]
}

/// Main stage to main stage: stop and play on Kodi, change input, and the receiver off and on.
pub(super) fn stages_script() -> Vec<(u64, Act)> {
    vec![
        (300, select_input(KODI)),
        (1800, button(Button::Stop)),
        (1600, button(Button::Play)),
        (1800, select_input(PLAYSTATION)),
        (1600, select_input(XBOX)),
        (1600, select_input(ANDROID_TV)),
        (1600, select_input(KODI)),
        (1600, power_key(true)),
        (1500, power_key(false)),
        (2000, power_key(true)),
        (1500, power_key(false)),
        (2200, ui(|_| {})),
    ]
}

/// Pick an input: the rows lift as focus moves, then the home screen shows the new input at once
/// with a sheen until the receiver confirms.
pub(super) fn switch_script() -> Vec<(u64, Act)> {
    vec![
        (900, ui(|a| a.go_to(PageId::Inputs, a.global::<Control>().get_input_index()))),
        (700, ui(|a| a.global::<Nav>().invoke_move(-1))),
        (600, ui(|a| a.global::<Nav>().invoke_move(-1))),
        (600, ui(|a| a.global::<Nav>().invoke_move(1))),
        (600, ui(|a| a.global::<Nav>().invoke_move(1))),
        (600, ui(|a| a.global::<Nav>().invoke_move(1))),
        (700, ui(|a| a.global::<Nav>().invoke_activate(a.global::<Nav>().get_focus()))),
        (3000, ui(|a| a.go_to(PageId::Inputs, a.global::<Control>().get_input_index()))),
        (700, ui(|a| a.global::<Nav>().invoke_move(-1))),
        (600, ui(|a| a.global::<Nav>().invoke_move(-1))),
        (600, ui(|a| a.global::<Nav>().invoke_move(-1))),
        (700, ui(|a| a.global::<Nav>().invoke_activate(a.global::<Nav>().get_focus()))),
        (3000, ui(|_| {})),
    ]
}

/// On the charger: the battery screen, then the countdown to dark, then a touch to wake.
pub(super) fn charge_script() -> Vec<(u64, Act)> {
    vec![
        (1200, key("c")),
        (3500, ui(|_| {})),
        // Nothing touched for ten seconds after it was put down: dark.
        (8500, ui(|_| {})),
        (2000, press(Button::VolumeUp.key_text())),
        (2500, ui(|_| {})),
    ]
}

pub(super) fn library_script() -> Vec<(u64, Act)> {
    let movies = Rc::new(shelf("movies"));
    let shows = Rc::new(shelf("tvshows"));
    let show = |shelf: &Rc<Shelf>, heading: &'static str, mode: LibMode| -> Act {
        let shelf = shelf.clone();
        Box::new(move |app, _| {
            app.global::<Lib>().set_heading(heading.into());
            app.global::<Lib>().set_count(shelf.count.clone().into());
            app.global::<Lib>().set_posters(Rc::new(slint::VecModel::from(shelf.posters.clone())).into());
            app.global::<Lib>().set_items(shelf.items());
            app.global::<Lib>().set_focus(0);
            app.global::<Lib>().set_mode(mode);
        })
    };
    let mut s: Vec<(u64, Act)> = vec![(300, show(&movies, "Movies", LibMode::River))];
    s.push((5000, show(&movies, "MOVIES", LibMode::RiverCard)));
    for _ in 0..4 {
        s.push((1500, focus_step(1)));
    }
    s.push((1800, show(&movies, "MOVIES", LibMode::Carousel)));
    for by in [1, 1, 1, -1, 1, 1] {
        s.push((1100, focus_step(by)));
    }
    s.push((1600, show(&movies, "MOVIES", LibMode::Wall)));
    for by in [1, 1, 3, 3, -1, 3, 3, 1] {
        s.push((900, focus_step(by)));
    }
    s.push((1600, show(&shows, "TV SHOWS", LibMode::DimRiverCard)));
    for _ in 0..4 {
        s.push((1500, focus_step(1)));
    }
    s.push((1800, Box::new(|_, _| {})));
    s
}

/// Into the poster wall from the idle home screen and back, as the live wall does it.
pub(super) fn wall_script() -> Vec<(u64, Act)> {
    let movies = Rc::new(shelf("movies"));
    let load: Act = {
        let shelf = movies.clone();
        Box::new(move |app, tx| {
            let _ = tx.send(UiAction::SelectInput(KODI.into()));
            shelf.list(app, "MOVIES");
            app.global::<Lib>().set_focus(4);
        })
    };
    let wall = |on: bool| -> Act { Box::new(move |app, _| crate::view::stage::library_stage(app, if on { LibMode::Wall } else { LibMode::Off })) };
    vec![(300, load), (600, button(Button::Stop)), (1500, wall(true)), (2200, focus_step(1)), (500, focus_step(3)), (900, wall(false)), (1600, wall(true)), (2200, ui(|a| crate::view::stage::reveal_library(a, Duration::ZERO))), (2200, ui(|_| {}))]
}

/// The poster wall settled on one film: the first whose title contains `UC_DEMO_FOCUS`.
pub(super) fn feature_script() -> Vec<(u64, Act)> {
    let movies = Rc::new(shelf("movies"));
    let focus = movies.focus();
    let load: Act = Box::new(move |app, tx| {
        let _ = tx.send(UiAction::SelectInput(KODI.into()));
        movies.list(app, "MOVIES");
        app.global::<Lib>().set_focus(focus);
    });
    vec![(300, load), (600, button(Button::Stop)), (1500, ui(|a| crate::view::stage::library_stage(a, LibMode::Wall))), (4000, ui(|_| {}))]
}

/// The wall's three ways of showing the highlighted title's details, one after another.
pub(super) fn walls_script() -> Vec<(u64, Act)> {
    let movies = Rc::new(shelf("movies"));
    let load: Act = {
        let shelf = movies.clone();
        Box::new(move |app, tx| {
            let _ = tx.send(UiAction::SelectInput(KODI.into()));
            shelf.list(app, "MOVIES");
        })
    };
    let style = |style: WallStyle| -> Act {
        Box::new(move |app, _| {
            crate::view::stage::library_stage(app, LibMode::Off);
            app.global::<Lib>().set_wall_style(style);
            app.global::<Lib>().set_focus(12);
        })
    };
    let wall: fn() -> Act = || Box::new(|app, _| crate::view::stage::library_stage(app, LibMode::Wall));
    let mut s: Vec<(u64, Act)> = vec![(300, load), (600, button(Button::Stop))];
    for n in [WallStyle::Sheet, WallStyle::Fade, WallStyle::Inline] {
        s.push((1200, style(n)));
        s.push((900, wall()));
        s.push((2000, focus_step(1)));
        s.push((1300, focus_step(3)));
        s.push((1300, focus_step(1)));
        s.push((1300, focus_step(-3)));
    }
    s.push((1800, ui(|_| {})));
    s
}

/// The subtitle picker with a long list: walk down past the screen and back.
pub(super) fn picker_script() -> Vec<(u64, Act)> {
    let open: Act = Box::new(|app, _| open_picker(app, Picker::Subtitles));
    let mut s: Vec<(u64, Act)> = vec![(600, open)];
    for _ in 0..16 {
        s.push((260, ui(|a| a.global::<Nav>().invoke_move(1))));
    }
    s.push((900, ui(|_| {})));
    for _ in 0..8 {
        s.push((260, ui(|a| a.global::<Nav>().invoke_move(-1))));
    }
    s.push((1200, ui(|_| {})));
    s
}

pub(super) fn lava_script() -> Vec<(u64, Act)> {
    vec![
        (200, select_input(KODI)),
        (4500, select_input(PLAYSTATION)),
        (4000, select_input(XBOX)),
        (4000, select_input(ANDROID_TV)),
        (4000, ui(|_| {})),
    ]
}

/// `--script steps:<file>`: a `--steps` script, filmed. Its pictures aren't taken; a failure to read it ends the
/// recording at once.
pub(super) fn steps_script(path: &str) -> Vec<(u64, Act)> {
    let timeline = crate::preview::steps::load(std::path::Path::new(path)).unwrap_or_else(|e| {
        tracing::error!("{path}: {e:#}");
        Vec::new()
    });
    // Like `--steps`, two seconds for the window and the drivers first.
    std::iter::once((2000, ui(|_| {})))
        .chain(timeline.into_iter().map(|(wait, action)| (wait.as_millis() as u64, Box::new(move |app: &AppWindow, _: &Tx| {
            crate::preview::steps::perform(app, &action, None);
        }) as Act)))
        .collect()
}

/// (pause before the step in ms, what happens)
pub(super) fn script(name: Option<&str>) -> Vec<(u64, Act)> {
    if let Some(path) = name.and_then(|n| n.strip_prefix("steps:")) {
        return steps_script(path);
    }
    match name {
        Some("media") => return media_script(),
        Some("charge") => return charge_script(),
        Some("lava") => return lava_script(),
        Some("layers") => return layers_script(),
        Some("stages") => return stages_script(),
        Some("switch") => return switch_script(),
        Some("library") => return library_script(),
        Some("picker") => return picker_script(),
        Some("wall") => return wall_script(),
        Some("feature") => return feature_script(),
        Some("walls") => return walls_script(),
        Some(n) if n.starts_with("site-") => return site_script(n),
        _ => {}
    }
    if name == Some("home") {
        return home_script();
    }
    if name == Some("wifi") {
        return wifi_script();
    }
    if name == Some("power") {
        return power_script();
    }
    if name == Some("hold") {
        return hold_script();
    }
    vec![
        // Receiver: nudge the volume.
        (1400, button(Button::VolumeUp)),
        (320, button(Button::VolumeUp)),
        (320, button(Button::VolumeUp)),
        // Open the input list, walk up to KODI, select it.
        (1300, ui(|a| a.go_to(PageId::Inputs, a.global::<Control>().get_input_index()))),
        (800, ui(|a| a.global::<Nav>().invoke_move(-1))),
        (380, ui(|a| a.global::<Nav>().invoke_move(-1))),
        (700, ui(|a| a.global::<Nav>().invoke_activate(a.global::<Nav>().get_focus()))),
        // Kodi is playing: volume overlay, mute, unmute.
        (2400, button(Button::VolumeUp)),
        (320, button(Button::VolumeUp)),
        (2600, button(Button::Mute)),
        (1500, button(Button::Mute)),
        // Stop (browsing), then play again.
        (2600, button(Button::Stop)),
        (2000, button(Button::Play)),
        // Menu → Display → brightness → back → Power → back out.
        (2000, ui(|a| a.go_to(PageId::Menu, 0))),
        (700, ui(|a| a.global::<Nav>().invoke_move(1))),
        (600, ui(|a| a.global::<Nav>().invoke_activate(1))),
        (900, ui(|a| a.global::<Prefs>().set_brightness(70))),
        (600, ui(|a| a.global::<Prefs>().set_brightness(45))),
        (700, ui(|a| a.global::<Prefs>().set_brightness(90))),
        (900, ui(|a| a.global::<Nav>().invoke_back())),
        (600, ui(|a| a.global::<Nav>().invoke_move(1))),
        (320, ui(|a| a.global::<Nav>().invoke_move(1))),
        (600, ui(|a| a.global::<Nav>().invoke_activate(3))),
        (1200, ui(|a| a.global::<Nav>().invoke_back())),
        (700, ui(|a| a.global::<Nav>().invoke_back())),
        // Switching input changes the interface colour, and the keys follow.
        (1400, ui(|a| a.global::<Control>().invoke_select_input(PLAYSTATION.into()))),
        (1700, ui(|a| a.global::<Control>().invoke_select_input(XBOX.into()))),
        (1700, ui(|a| a.global::<Control>().invoke_select_input(ANDROID_TV.into()))),
        (1700, ui(|a| a.global::<Control>().invoke_select_input(KODI.into()))),
        (1600, ui(|_| {})),
    ]
}
