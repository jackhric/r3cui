use super::acts::{unix_now, Tx};
use super::keys;
use super::scripts::script;
use crate::hardware::backlight;
use crate::preview::device::{self, Device};
use crate::AppWindow;
use anyhow::Context;
use slint::ComponentHandle;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Starts the recording; the returned timer must be kept alive until the event loop ends.
pub fn record(app: &AppWindow, tx: Tx, mut device: Device, out: PathBuf, which: Option<&str>) -> anyhow::Result<slint::Timer> {
    let mut ffmpeg = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgba"])
        .args(["-s", &format!("{}x{}", device::WIDTH, device.height())])
        .args(["-framerate", &FPS.to_string(), "-i", "-"])
        // A `.png` pattern such as `frames/%04d.png` keeps every frame lossless, for stills.
        .args(match out.extension().and_then(|e| e.to_str()) {
            Some("png") => &[][..],
            // Lossless with alpha, and quicker to write than PNGs.
            Some("mkv") => &["-c:v", "ffv1", "-level", "3", "-threads", "4", "-slices", "16"][..],
            _ => &["-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "20", "-movflags", "+faststart"][..],
        })
        .arg(&out)
        .stdin(Stdio::piped())
        .spawn()
        .context("starting ffmpeg")?;
    let mut pipe = ffmpeg.stdin.take().context("ffmpeg stdin")?;

    // The UI thread only grabs the screen, often; composing onto the remote and writing happen here. Each
    // output frame shows the latest grab taken at or before its moment, so the video keeps real time evenly
    // whatever a grab costs.
    let (grabs, grabbed) = std::sync::mpsc::sync_channel::<(Instant, Vec<u8>, (u32, u32), backlight::Keys, f32)>(4);
    let writer = std::thread::spawn(move || {
        let mut start = None;
        let mut written = 0u64;
        let mut last: Option<image::RgbaImage> = None;
        let mut write = |frame: &image::RgbaImage| {
            let _ = pipe.write_all(frame.as_raw());
        };
        for (at, pixels, size, keys, dt) in grabbed {
            let frame = device.compose(&pixels, size, keys, dt);
            let start = *start.get_or_insert(at);
            while start + Duration::from_secs_f64(written as f64 / FPS as f64) < at {
                write(last.as_ref().unwrap_or(&frame));
                written += 1;
            }
            last = Some(frame);
        }
        if let Some(frame) = &last {
            write(frame);
        }
        // Closing the pipe is what tells ffmpeg the stream has ended.
        drop(write);
        drop(pipe);
        ffmpeg.wait()
    });

    let mut steps = script(which).into_iter();
    let mut pending = steps.next();
    let mut since = Instant::now();
    let mut last_frame = Instant::now();
    let mut first = true;
    let mut grabs = Some(grabs);
    let mut writer = Some(writer);
    let weak = app.as_weak();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(15), move || {
        let Some(app) = weak.upgrade() else { return };
        if pending.as_ref().is_some_and(|(wait, _)| since.elapsed() >= Duration::from_millis(*wait)) {
            if let Some((_, act)) = pending.take() {
                act(&app, &tx);
                println!("record step {:.3}", unix_now());
            }
            since = Instant::now();
            pending = steps.next();
            if pending.is_none() {
                drop(grabs.take());
                if let Some(writer) = writer.take() {
                    println!("{} {:?}", out.display(), writer.join().map_err(|_| "writer panicked"));
                }
                let _ = slint::quit_event_loop();
                return;
            }
        }
        if let (Some(grabs), Ok(buf)) = (&grabs, app.window().take_snapshot()) {
            let at = Instant::now();
            // Printed once, so a screen recorded elsewhere at the same time can be lined up with this one.
            if first {
                first = false;
                println!("record t0 {:.3}", unix_now());
            }
            let dt = last_frame.elapsed().as_secs_f32();
            last_frame = at;
            let _ = grabs.send((at, buf.as_bytes().to_vec(), (buf.width(), buf.height()), keys(&app), dt));
        }
    });
    Ok(timer)
}

pub(super) const FPS: u32 = 30;
