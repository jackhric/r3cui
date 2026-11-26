//! A stand-in Android TV for the desktop UI: `cargo run -p androidtv --features testing --example fake-androidtv`.
//! It listens on 127.0.0.1 and prints the pairing code whenever a client starts pairing.

use androidtv::testing::{FakeOptions, FakeTv};
use androidtv::{PAIRING_PORT, REMOTE_PORT};

#[tokio::main]
async fn main() {
    let tv = FakeTv::start_with(FakeOptions { remote_addr: format!("127.0.0.1:{REMOTE_PORT}"), pairing_addr: format!("127.0.0.1:{PAIRING_PORT}"), ..FakeOptions::default() }).await;
    println!("FakeTv on 127.0.0.1 (remote {REMOTE_PORT}, pairing {PAIRING_PORT})");
    let mut code = tv.code();
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return,
            changed = code.changed() => {
                if changed.is_err() {
                    return;
                }
                if let Some(code) = code.borrow().as_deref() {
                    println!("pairing code: {code}");
                }
            }
        }
    }
}
