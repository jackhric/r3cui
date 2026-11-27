//! Pair with a TV: `cargo run -p androidtv --example atv-pair -- <host> [credentials.json]`.
//! Type the code the TV shows; the client certificate is saved for `atv-watch`.

use androidtv::{pair, Credentials};
use std::io::{BufRead, Write};

const CLIENT_NAME: &str = "r3cui";
const DEFAULT_FILE: &str = "androidtv-credentials.json";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let host = args.next().ok_or("usage: atv-pair <host> [credentials.json]")?;
    let file = args.next().unwrap_or_else(|| DEFAULT_FILE.into());

    let credentials = match std::fs::read_to_string(&file) {
        Ok(json) => serde_json::from_str(&json)?,
        Err(_) => {
            println!("generating a client certificate (a few seconds)...");
            tokio::task::spawn_blocking(Credentials::generate).await??
        }
    };

    let mut pairing = pair(host.as_str(), &credentials, CLIENT_NAME).await?;
    loop {
        print!("code shown on the TV: ");
        std::io::stdout().flush()?;
        let mut code = String::new();
        if std::io::stdin().lock().read_line(&mut code)? == 0 {
            return Err("no code given".into());
        }
        match pairing.finish(&code).await {
            Ok(()) => break,
            Err(androidtv::Error::WrongCode | androidtv::Error::InvalidCode) => println!("that code does not match, try again"),
            Err(e) => return Err(e.into()),
        }
    }
    std::fs::write(&file, serde_json::to_string_pretty(&credentials)?)?;
    println!("paired; credentials saved to {file}");
    Ok(())
}
