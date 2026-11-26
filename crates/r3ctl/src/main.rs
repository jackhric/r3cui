//! r3ctl: one request to r3cui's remote control, the reply printed as JSON.
//!
//!     r3ctl state
//!     r3ctl tap ok | press voice | release voice | hold home 1200
//!     r3ctl touch 240 400 | swipe 240 600 240 200 [ms] | slide 3 | page menu
//!     r3ctl screenshot [-o shot.png]
//!
//! `R3CTL_HOST` is the remote (`ip` or `ip:port`, default 127.0.0.1 for the desktop demo), `R3CTL_KEY` the access key
//! shown in Settings → System → Remote control.

use anyhow::{anyhow, bail, Context};
use futures_util::{SinkExt, StreamExt};
use remotewire::{Challenge, Envelope, HardKey, Hello, KeyAction, Point, Reply, Request};
use tokio_tungstenite::tungstenite::Message;

const HOST_ENV: &str = "R3CTL_HOST";
const KEY_ENV: &str = "R3CTL_KEY";
const DEFAULT_SWIPE_MS: u32 = 250;
const DEFAULT_SHOT: &str = "r3cui.png";

fn num<T: std::str::FromStr>(arg: Option<&String>, what: &str) -> anyhow::Result<T> {
    arg.ok_or_else(|| anyhow!("missing {what}"))?.parse().map_err(|_| anyhow!("{what} is not a number"))
}

fn hard_key(arg: Option<&String>) -> anyhow::Result<HardKey> {
    let name = arg.ok_or_else(|| anyhow!("missing key"))?;
    HardKey::from_name(name).ok_or_else(|| anyhow!("no key {name:?}; keys: {}", HardKey::ALL.map(HardKey::name).join(" ")))
}

fn point(x: Option<&String>, y: Option<&String>) -> anyhow::Result<Point> {
    Ok(Point { x: num(x, "x")?, y: num(y, "y")? })
}

/// The request, and where a screenshot goes.
fn parse(args: &[String]) -> anyhow::Result<(Request, Option<String>)> {
    let a = |i: usize| args.get(i);
    let key = |action| Ok::<_, anyhow::Error>(Request::Key { key: hard_key(a(1))?, action });
    let request = match args.first().map(String::as_str) {
        Some("state") => Request::State,
        Some("press") => key(KeyAction::Press)?,
        Some("release") => key(KeyAction::Release)?,
        Some("tap") => key(KeyAction::Tap)?,
        Some("hold") => key(KeyAction::Hold { ms: num(a(2), "milliseconds")? })?,
        Some("touch") => Request::Tap { at: point(a(1), a(2))? },
        Some("swipe") => Request::Swipe { from: point(a(1), a(2))?, to: point(a(3), a(4))?, ms: a(5).map_or(Ok(DEFAULT_SWIPE_MS), |_| num(a(5), "milliseconds"))? },
        Some("slide") => Request::Slide { steps: num(a(1), "steps")? },
        Some("page") => Request::Page { name: a(1).ok_or_else(|| anyhow!("missing page"))?.clone() },
        Some("screenshot") => {
            let out = match (a(1).map(String::as_str), a(2)) {
                (Some("-o"), Some(path)) => path.clone(),
                _ => DEFAULT_SHOT.to_string(),
            };
            return Ok((Request::Screenshot, Some(out)));
        }
        _ => bail!("usage: r3ctl state | press|release|tap <key> | hold <key> <ms> | touch <x> <y> | swipe <x> <y> <x> <y> [ms] | slide <steps> | page <name> | screenshot [-o file]"),
    };
    Ok((request, None))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (request, out) = parse(&args)?;
    let host = std::env::var(HOST_ENV).unwrap_or_else(|_| "127.0.0.1".into());
    let host = if host.contains(':') { host } else { format!("{host}:{}", remotewire::PORT) };
    let key = std::env::var(KEY_ENV).with_context(|| format!("set {KEY_ENV} to the key shown on the remote"))?;

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{host}{}", remotewire::PATH)).await.with_context(|| format!("connecting to {host}"))?;
    let first = ws.next().await.ok_or_else(|| anyhow!("closed before the challenge"))??;
    let Challenge::Challenge { nonce } = serde_json::from_str(first.to_text()?)?;
    ws.send(Message::text(serde_json::to_string(&Hello::Hello { proof: remotewire::proof(&key, &nonce) })?)).await?;
    ws.send(Message::text(serde_json::to_string(&Envelope { id: 1, body: request })?)).await?;

    let reply = loop {
        match ws.next().await.ok_or_else(|| anyhow!("closed without a reply (wrong key?)"))?? {
            Message::Text(text) => break serde_json::from_str::<Envelope<Reply>>(&text)?,
            Message::Close(_) => bail!("closed without a reply (wrong key?)"),
            _ => {}
        }
    };
    println!("{}", serde_json::to_string(&reply.body)?);
    if let (Reply::Screenshot { .. }, Some(out)) = (&reply.body, out) {
        match ws.next().await.ok_or_else(|| anyhow!("no image followed"))?? {
            Message::Binary(png) => {
                std::fs::write(&out, &png).with_context(|| format!("writing {out}"))?;
                eprintln!("saved {out}");
            }
            other => bail!("expected the image, got {other:?}"),
        }
    }
    let _ = ws.close(None).await;
    if let Reply::Error { message } = reply.body {
        bail!(message);
    }
    Ok(())
}
