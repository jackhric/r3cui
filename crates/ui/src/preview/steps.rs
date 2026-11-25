//! `--steps <file> --out <dir>`: drive the live app through its own window, as a person with the remote would, and
//! take pictures along the way, for checking a change end to end. One step per line:
//!
//! ```text
//! wait 800          # milliseconds
//! key down          # up down left right ok back home menu input, or a desktop key's character
//! hold home 900     # held for that long (home and power act on a hold)
//! snap inputs       # <out>/inputs.png
//! awake             # put away the charger card the simulator always shows
//! quit
//! ```

use crate::AppWindow;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, SharedString};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

enum Step {
    Wait(Duration),
    Key(SharedString),
    Hold(SharedString, Duration),
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
        "home" => return "h".into(),
        "menu" => return "m".into(),
        "input" => return "v".into(),
        "power" => return "p".into(),
        other => return other.into(),
    };
    char::from(k).into()
}

fn parse(text: &str) -> Result<Vec<Step>, String> {
    let mut steps = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut words = line.split_whitespace();
        let Some(verb) = words.next() else { continue };
        let arg = words.next();
        let ms = |a: Option<&str>| a.and_then(|a| a.parse().ok()).map(Duration::from_millis).ok_or(format!("line {}: needs milliseconds", n + 1));
        steps.push(match verb {
            "wait" => Step::Wait(ms(arg)?),
            "key" => Step::Key(key_text(arg.ok_or(format!("line {}: which key", n + 1))?)),
            "hold" => Step::Hold(key_text(arg.ok_or(format!("line {}: which key", n + 1))?), ms(words.next())?),
            "snap" => Step::Snap(arg.ok_or(format!("line {}: snap needs a name", n + 1))?.to_string()),
            "awake" => Step::Awake,
            "quit" => Step::Quit,
            other => return Err(format!("line {}: no step {other:?}", n + 1)),
        });
    }
    Ok(steps)
}

fn snap(app: &AppWindow, path: &Path) {
    let saved = app.window().take_snapshot().map_err(anyhow::Error::from).and_then(|buf| {
        image::save_buffer(path, buf.as_bytes(), buf.width(), buf.height(), image::ColorType::Rgba8)?;
        Ok(())
    });
    println!("{} {saved:?}", path.display());
}

pub fn run(app: &AppWindow, script: &Path, out: PathBuf) -> anyhow::Result<slint::Timer> {
    let steps = parse(&std::fs::read_to_string(script)?).map_err(anyhow::Error::msg)?;
    std::fs::create_dir_all(&out)?;
    let weak = app.as_weak();
    let (mut at, mut due, mut release) = (0usize, Instant::now() + Duration::from_secs(2), None::<(SharedString, Instant)>);
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(20), move || {
        let Some(app) = weak.upgrade() else { return };
        let now = Instant::now();
        if let Some((text, when)) = release.take() {
            if now < when {
                release = Some((text, when));
                return;
            }
            app.window().dispatch_event(WindowEvent::KeyReleased { text });
        }
        while now >= due {
            let Some(step) = steps.get(at) else {
                let _ = slint::quit_event_loop();
                return;
            };
            at += 1;
            match step {
                Step::Wait(d) => due = now + *d,
                Step::Key(text) => {
                    app.window().dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
                    release = Some((text.clone(), now + Duration::from_millis(60)));
                    due = now + Duration::from_millis(80);
                    return;
                }
                Step::Hold(text, d) => {
                    app.window().dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
                    release = Some((text.clone(), now + *d));
                    due = now + *d + Duration::from_millis(20);
                    return;
                }
                Step::Snap(name) => snap(&app, &out.join(format!("{name}.png"))),
                Step::Awake => {
                    app.set_charge_card(false);
                    app.set_resting(false);
                }
                Step::Quit => {
                    let _ = slint::quit_event_loop();
                    return;
                }
            }
        }
    });
    Ok(timer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_parse() {
        let steps = parse("awake\nwait 500 # settle\nkey down\nhold home 900\nsnap x\n\nquit").unwrap();
        assert_eq!(steps.len(), 6);
        assert!(matches!(&steps[2], Step::Key(t) if t.as_str() == char::from(Key::DownArrow).to_string()));
        assert!(parse("jump 3").unwrap_err().contains("line 1"));
        assert!(parse("wait soon").is_err());
    }
}
