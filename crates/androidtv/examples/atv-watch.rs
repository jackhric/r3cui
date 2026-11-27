//! Follow a paired TV: `cargo run -p androidtv --example atv-watch -- <host> [credentials.json] [key]`.
//! Prints the state as it changes; with a key (home, back, up, down, left, right, ok, playpause, power, volup,
//! voldown) it presses that key once the TV is ready.

use androidtv::{connect, Credentials, KeyCode, Link};

const DEFAULT_FILE: &str = "androidtv-credentials.json";

fn key(name: &str) -> Option<KeyCode> {
    Some(match name {
        "home" => KeyCode::Home,
        "back" => KeyCode::Back,
        "up" => KeyCode::DpadUp,
        "down" => KeyCode::DpadDown,
        "left" => KeyCode::DpadLeft,
        "right" => KeyCode::DpadRight,
        "ok" => KeyCode::DpadCenter,
        "playpause" => KeyCode::MediaPlayPause,
        "power" => KeyCode::Power,
        "volup" => KeyCode::VolumeUp,
        "voldown" => KeyCode::VolumeDown,
        _ => return None,
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let host = args.next().ok_or("usage: atv-watch <host> [credentials.json] [key]")?;
    let file = args.next().unwrap_or_else(|| DEFAULT_FILE.into());
    let pressed = args.next().map(|name| key(&name).ok_or(format!("unknown key {name}"))).transpose()?;

    let credentials: Credentials = serde_json::from_str(&std::fs::read_to_string(&file)?)?;
    let tv = connect(host.as_str(), &credentials)?;
    let mut state = tv.watch();
    let mut sent = pressed.is_none();
    loop {
        println!("{:?}", *state.borrow_and_update());
        if !sent && tv.state().link == Link::Up {
            sent = true;
            tv.press(pressed.unwrap())?;
            println!("pressed {:?}", pressed.unwrap());
        }
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            changed = state.changed() => changed?,
        }
    }
}
