//! Remote control: a small server inside r3cui so the remote can be watched and driven from the local network. Off
//! unless `R3CUI_REMOTE_CONTROL` is set (in `settings.json`'s `env`, say). It does nothing until asked: no polling, and
//! the screen is captured only when a screenshot is requested, so a remote on battery pays only for what is used.
//!
//! First step: `GET /health` and `GET /screenshot.png`, with how long the capture and the encoding took.

use crate::AppWindow;
use image::ImageEncoder;
use slint::ComponentHandle;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const ENV: &str = "R3CUI_REMOTE_CONTROL";
/// Outside the ports a custom integration may not use, and beside the voice link (10790).
pub(crate) const PORT: u16 = 10780;
const REQUEST_LIMIT: usize = 4096;
const READ_TIMEOUT: Duration = Duration::from_secs(5);

mod route {
    pub const HEALTH: &str = "/health";
    pub const SCREENSHOT: &str = "/screenshot.png";
}

/// Starts the server on its own thread when switched on.
pub(crate) fn start(weak: slint::Weak<AppWindow>) {
    if std::env::var_os(ENV).is_none() {
        return;
    }
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => return tracing::error!("remote control: {e}"),
        };
        rt.block_on(serve(weak));
    });
}

async fn serve(weak: slint::Weak<AppWindow>) {
    let addr = SocketAddr::from(([0, 0, 0, 0], PORT));
    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => return tracing::error!("remote control: cannot listen on {addr}: {e}"),
    };
    tracing::info!("remote control: listening on {addr}");
    loop {
        let Ok((stream, peer)) = listener.accept().await else { continue };
        if !local(peer.ip()) {
            tracing::warn!("remote control: refused {peer}: not on the local network");
            continue;
        }
        let weak = weak.clone();
        tokio::spawn(async move {
            if let Err(e) = answer(stream, weak).await {
                tracing::warn!("remote control: {peer}: {e}");
            }
        });
    }
}

/// Loopback, link-local or a private range: never an address from the internet.
fn local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unicast_link_local() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_private() || v4.is_loopback()),
    }
}

async fn answer(mut stream: TcpStream, weak: slint::Weak<AppWindow>) -> anyhow::Result<()> {
    let mut buf = vec![0; REQUEST_LIMIT];
    let n = tokio::time::timeout(READ_TIMEOUT, stream.read(&mut buf)).await??;
    let request = String::from_utf8_lossy(&buf[..n]);
    let path = request.lines().next().and_then(|line| line.split_whitespace().nth(1)).unwrap_or("");
    match path {
        route::HEALTH => respond(&mut stream, "200 OK", "text/plain", &[], b"r3cui\n").await,
        route::SCREENSHOT => {
            let (png, capture, encode) = screenshot(weak).await?;
            let timing = [("X-Capture-Ms", millis(capture)), ("X-Encode-Ms", millis(encode))];
            tracing::info!("remote control: screenshot, capture {} ms, encode {} ms, {} KB", timing[0].1, timing[1].1, png.len() / 1024);
            respond(&mut stream, "200 OK", "image/png", &timing, &png).await
        }
        _ => respond(&mut stream, "404 Not Found", "text/plain", &[], b"not found\n").await,
    }
}

fn millis(d: Duration) -> String {
    format!("{:.1}", d.as_secs_f64() * 1000.0)
}

/// Captured on the UI thread, which owns the window, and encoded here, off it.
async fn screenshot(weak: slint::Weak<AppWindow>) -> anyhow::Result<(Vec<u8>, Duration, Duration)> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    weak.upgrade_in_event_loop(move |app| {
        let start = Instant::now();
        let shot = app.window().take_snapshot();
        let _ = tx.send((shot, start.elapsed()));
    })?;
    let (shot, capture) = rx.await?;
    let shot = shot?;
    let start = Instant::now();
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(&mut png, image::codecs::png::CompressionType::Fast, image::codecs::png::FilterType::Adaptive)
        .write_image(shot.as_bytes(), shot.width(), shot.height(), image::ExtendedColorType::Rgba8)?;
    Ok((png, capture, start.elapsed()))
}

async fn respond(stream: &mut TcpStream, status: &str, kind: &str, headers: &[(&str, String)], body: &[u8]) -> anyhow::Result<()> {
    let mut head = format!("HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    Ok(())
}
