//! Steps: drive the live app through its own window, as a person with the remote would. `--steps <file> --out <dir>`
//! plays them and takes pictures along the way; `--record <video> --script steps:<file>` films them. One per line:
//!
//! ```text
//! wait 800          # milliseconds
//! key down          # up down left right ok back home menu input power, or a desktop key's character
//! hold home 900     # held for that long (home and power act on a hold)
//! type 127.0.0.1    # tapped out on the on-screen keyboard, a key at a time
//! tap Done          # one keyboard key, by its label
//! page menu        # open a page by name (home inputs menu display power wifi keyboard status join-failed tracks
//!                   # feedback-lab integrations), as a tap on its button would
//! snap inputs       # <out>/inputs.png (not when filming)
//! awake             # put away the charger card the simulator always shows
//! quit
//! ```

use crate::view::screen::{Screen, page_by_name};
use crate::{AppWindow, PageId};
use crate::input::keys;
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model as _, SharedString};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A key's press and release.
const PRESS: Duration = Duration::from_millis(60);
/// Between two keys tapped out by `type`, about a quick finger's pace.
const TYPING: Duration = Duration::from_millis(260);

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Do {
    Press(SharedString),
    Release(SharedString),
    /// One keyboard key, by its label; the keyboard's mode is switched first if it isn't showing.
    Tap(String),
    Page(PageId),
    Snap(String),
    Awake,
    Quit,
}

fn key_text(name: &str) -> SharedString {
    let k = match name {
        "up" => Key::UpArrow,
        "down" => Key::DownArrow,
        "left" => Key::LeftArrow,
        "right" => Key::RightArrow,
        "ok" => Key::Return,
        "back" => Key::Escape,
        "home" => return keys::HOME.into(),
        "menu" => return keys::MENU.into(),
        "input" => return keys::INPUT.into(),
        "power" => return keys::POWER.into(),
        other => return other.into(),
    };
    char::from(k).into()
}

/// The script as a timeline: each action with the time to wait before it.
pub(crate) fn parse(text: &str) -> Result<Vec<(Duration, Do)>, String> {
    let mut out = Vec::new();
    let mut wait = Duration::ZERO;
    let mut push = |wait: &mut Duration, d: Do| out.push((std::mem::take(wait), d));
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        let (verb, rest) = line.split_once(' ').map_or((line, ""), |(v, r)| (v, r.trim()));
        let err = |what: &str| format!("line {}: {verb} needs {what}", n + 1);
        let ms = |a: Option<&str>| a.and_then(|a| a.parse().ok()).map(Duration::from_millis);
        match verb {
            "" => {}
            "wait" => wait += ms(Some(rest)).ok_or(err("milliseconds"))?,
            "key" | "hold" => {
                let mut words = rest.split_whitespace();
                let text = key_text(words.next().ok_or(err("a key"))?);
                let held = if verb == "hold" { ms(words.next()).ok_or(err("milliseconds"))? } else { PRESS };
                push(&mut wait, Do::Press(text.clone()));
                wait = held;
                push(&mut wait, Do::Release(text));
                wait = Duration::from_millis(20);
            }
            "type" => {
                for c in rest.chars() {
                    push(&mut wait, Do::Tap(c.to_string()));
                    wait = TYPING;
                }
            }
            "tap" => push(&mut wait, Do::Tap(rest.to_string())),
            "page" => push(&mut wait, Do::Page(page_by_name(rest).ok_or(err("a page name"))?)),
            "snap" if !rest.is_empty() => push(&mut wait, Do::Snap(rest.to_string())),
            "snap" => return Err(err("a name")),
            "awake" => push(&mut wait, Do::Awake),
            "quit" => push(&mut wait, Do::Quit),
            other => return Err(format!("line {}: no step {other:?}", n + 1)),
        }
    }
    Ok(out)
}

pub(crate) fn load(script: &Path) -> anyhow::Result<Vec<(Duration, Do)>> {
    parse(&std::fs::read_to_string(script)?).map_err(anyhow::Error::msg)
}

/// Where a keyboard key with this label is on screen, if it is.
fn key_at(app: &AppWindow, label: &str) -> Option<LogicalPosition> {
    let keys = app.get_kb_keys();
    let found = keys.iter().find(|k| k.label == label || (label.len() == 1 && k.label.eq_ignore_ascii_case(label)))?;
    Some(LogicalPosition::new(found.x + found.w / 2.0, found.y + found.h / 2.0))
}

fn touch(app: &AppWindow, at: LogicalPosition) {
    let w = app.window();
    w.dispatch_event(WindowEvent::PointerMoved { position: at });
    w.dispatch_event(WindowEvent::PointerPressed { position: at, button: PointerEventButton::Left });
    w.dispatch_event(WindowEvent::PointerReleased { position: at, button: PointerEventButton::Left });
}

/// Does one action; `false` when it was the last.
pub(crate) fn perform(app: &AppWindow, action: &Do, out: Option<&Path>) -> bool {
    match action {
        Do::Press(text) => app.window().dispatch_event(WindowEvent::KeyPressed { text: text.clone() }),
        Do::Release(text) => app.window().dispatch_event(WindowEvent::KeyReleased { text: text.clone() }),
        Do::Tap(label) => {
            // Digits and symbols live on the other layout; capitals need shift, which a script doesn't ask for.
            let at = key_at(app, label).or_else(|| {
                touch(app, key_at(app, "123").or_else(|| key_at(app, "abc"))?);
                key_at(app, label)
            });
            match at {
                Some(at) => touch(app, at),
                None => tracing::warn!("steps: no key {label:?} on the keyboard"),
            }
        }
        Do::Page(page) => app.go_to(*page, 0),
        Do::Snap(name) => {
            if let Some(dir) = out {
                snap(app, &dir.join(format!("{name}.png")));
            }
        }
        Do::Awake => {
            app.set_charge_card(false);
            app.set_resting(false);
        }
        Do::Quit => return false,
    }
    true
}

fn snap(app: &AppWindow, path: &Path) {
    let saved = app.window().take_snapshot().map_err(anyhow::Error::from).and_then(|buf| {
        image::save_buffer(path, buf.as_bytes(), buf.width(), buf.height(), image::ColorType::Rgba8)?;
        Ok(())
    });
    println!("{} {saved:?}", path.display());
}

/// `--steps`: plays the script, taking its pictures into `out`, then quits.
pub fn run(app: &AppWindow, script: &Path, out: PathBuf) -> anyhow::Result<slint::Timer> {
    let mut timeline = load(script)?.into_iter();
    std::fs::create_dir_all(&out)?;
    let weak = app.as_weak();
    // Two seconds for the window to come up and the drivers to connect.
    let mut due = Instant::now() + Duration::from_secs(2);
    let mut next = timeline.next();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(10), move || {
        let Some(app) = weak.upgrade() else { return };
        while let Some((wait, action)) = &next {
            if Instant::now() < due + *wait {
                return;
            }
            due += *wait;
            if !perform(&app, action, Some(&out)) {
                break;
            }
            next = timeline.next();
        }
        let _ = slint::quit_event_loop();
    });
    Ok(timer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_become_a_timeline() {
        let t = parse("awake\nwait 500 # settle\nkey down\nhold home 900\ntype a.1\nsnap x\n\nquit").unwrap();
        let down = key_text("down");
        assert_eq!(t[0], (Duration::ZERO, Do::Awake));
        assert_eq!(t[1], (Duration::from_millis(500), Do::Press(down.clone())));
        assert_eq!(t[2], (PRESS, Do::Release(down)));
        assert_eq!(t[3], (Duration::from_millis(20), Do::Press("h".into())));
        assert_eq!(t[4], (Duration::from_millis(900), Do::Release("h".into())));
        assert_eq!(t[5..8].iter().map(|(_, d)| d.clone()).collect::<Vec<_>>(), [Do::Tap("a".into()), Do::Tap(".".into()), Do::Tap("1".into())]);
        assert_eq!(t[6].0, TYPING);
        assert_eq!(t.last(), Some(&(Duration::ZERO, Do::Quit)));
        assert!(parse("jump 3").unwrap_err().contains("line 1"));
        assert!(parse("wait soon").is_err());
    }
}
