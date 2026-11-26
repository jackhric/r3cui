//! What each request does: keys and touches go in through the same door as real ones (`input::inject`).

use crate::app::UiAction;
use crate::input::inject::deliver;
use crate::input::slider::Gesture;
use crate::input::{keys, Button};
use crate::view::screen::page_by_name;
use crate::view::screen::Screen as _;
use crate::{AppWindow, Control, Nav, RemoteControl, Sys, Voice};
use image::ImageEncoder;
use remotewire::{HardKey, KeyAction, Point, Reply, Request, State};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, SharedString};
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// How long a tap holds, a key's or a finger's.
const TAP: Duration = Duration::from_millis(80);
/// How long to wait for an asleep remote to wake before the input goes in anyway, and how often to look.
const WAKE_WAIT: Duration = Duration::from_secs(2);
const WAKE_LOOK: Duration = Duration::from_millis(100);
/// A swipe's finger moves this often.
const SWIPE_STEP: Duration = Duration::from_millis(16);

static ATTACHED: AtomicI32 = AtomicI32::new(0);

/// A client came or went: the remote shows while anyone is in.
pub(super) fn attached(weak: &slint::Weak<crate::AppWindow>, change: i32) {
    let now = ATTACHED.fetch_add(change, Ordering::SeqCst) + change;
    let _ = weak.upgrade_in_event_loop(move |app| app.global::<RemoteControl>().set_watched(now > 0));
}

fn key_text(key: HardKey) -> SharedString {
    let button = |b: Button| b.key_text();
    match key {
        HardKey::Back => button(Button::Back),
        HardKey::Home => keys::HOME.into(),
        HardKey::Menu => button(Button::Menu),
        HardKey::Up => button(Button::DpadUp),
        HardKey::Down => button(Button::DpadDown),
        HardKey::Left => button(Button::DpadLeft),
        HardKey::Right => button(Button::DpadRight),
        HardKey::Ok => button(Button::DpadMiddle),
        HardKey::VolumeUp => button(Button::VolumeUp),
        HardKey::VolumeDown => button(Button::VolumeDown),
        HardKey::Mute => button(Button::Mute),
        HardKey::ChannelUp => button(Button::ChannelUp),
        HardKey::ChannelDown => button(Button::ChannelDown),
        HardKey::Prev => button(Button::Prev),
        HardKey::Play => button(Button::Play),
        HardKey::Next => button(Button::Next),
        HardKey::Stop => button(Button::Stop),
        HardKey::Record => button(Button::Record),
        HardKey::Voice => keys::INPUT.into(),
    }
}

fn position(p: Point) -> LogicalPosition {
    LogicalPosition::new(p.x, p.y)
}

async fn key(weak: &slint::Weak<AppWindow>, key: HardKey, action: KeyAction) {
    let text = key_text(key);
    let press = || deliver(weak, WindowEvent::KeyPressed { text: text.clone() });
    let release = || deliver(weak, WindowEvent::KeyReleased { text: text.clone() });
    match action {
        KeyAction::Press => press(),
        KeyAction::Release => release(),
        KeyAction::Tap => {
            press();
            tokio::time::sleep(TAP).await;
            release();
        }
        KeyAction::Hold { ms } => {
            press();
            tokio::time::sleep(Duration::from_millis(u64::from(ms))).await;
            release();
        }
    }
}

async fn swipe(weak: &slint::Weak<AppWindow>, from: Point, to: Point, ms: u32) {
    let button = PointerEventButton::Left;
    deliver(weak, WindowEvent::PointerMoved { position: position(from) });
    deliver(weak, WindowEvent::PointerPressed { position: position(from), button });
    let steps = (u64::from(ms) / SWIPE_STEP.as_millis() as u64).max(1);
    for s in 1..=steps {
        tokio::time::sleep(SWIPE_STEP).await;
        let f = s as f32 / steps as f32;
        let at = Point { x: from.x + (to.x - from.x) * f, y: from.y + (to.y - from.y) * f };
        deliver(weak, WindowEvent::PointerMoved { position: position(at) });
    }
    deliver(weak, WindowEvent::PointerReleased { position: position(to), button });
    deliver(weak, WindowEvent::PointerExited);
}

/// Runs on the UI thread and hands back what `f` returns.
async fn on_ui<T: Send + 'static>(weak: &slint::Weak<AppWindow>, f: impl FnOnce(AppWindow) -> T + Send + 'static) -> anyhow::Result<T> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    weak.upgrade_in_event_loop(move |app| drop(tx.send(f(app))))?;
    Ok(rx.await?)
}

fn state(app: &AppWindow) -> State {
    let page = crate::view::screen::page_name(app.global::<Nav>().get_page()).to_string();
    State {
        page,
        focus: app.global::<Nav>().get_focus(),
        input: app.global::<Control>().get_input().to_string(),
        voice: format!("{:?}", app.global::<Voice>().get_stage()).to_lowercase(),
        awake: app.global::<Sys>().get_awake(),
        battery: app.global::<Sys>().get_battery(),
        charging: app.global::<Sys>().get_charging(),
    }
}

/// Captured on the UI thread, which owns the window, and encoded here, off it.
async fn screenshot(weak: &slint::Weak<AppWindow>) -> anyhow::Result<(Reply, Vec<u8>)> {
    let (shot, capture) = on_ui(weak, |app| {
        let start = Instant::now();
        (app.window().take_snapshot(), start.elapsed())
    })
    .await?;
    let shot = shot?;
    let start = Instant::now();
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(&mut png, image::codecs::png::CompressionType::Fast, image::codecs::png::FilterType::Adaptive)
        .write_image(shot.as_bytes(), shot.width(), shot.height(), image::ExtendedColorType::Rgba8)?;
    let encode = start.elapsed();
    let reply = Reply::Screenshot { width: shot.width(), height: shot.height(), bytes: png.len() as u64, capture_ms: capture.as_secs_f32() * 1000.0, encode_ms: encode.as_secs_f32() * 1000.0 };
    Ok((reply, png))
}

/// A real key wakes the remote through the core; virtual input has to ask for it, then wait for the screen.
async fn wake(weak: &slint::Weak<AppWindow>, actions: &mpsc::UnboundedSender<UiAction>) {
    let awake = || on_ui(weak, |app| app.global::<Sys>().get_awake());
    if awake().await.unwrap_or(true) {
        return;
    }
    let _ = actions.send(UiAction::Wake);
    let until = Instant::now() + WAKE_WAIT;
    while Instant::now() < until && !awake().await.unwrap_or(true) {
        tokio::time::sleep(WAKE_LOOK).await;
    }
}

/// Does what is asked; a screenshot also hands back its PNG.
pub(super) async fn run(request: Request, weak: &slint::Weak<AppWindow>, actions: &mpsc::UnboundedSender<UiAction>) -> (Reply, Option<Vec<u8>>) {
    let error = |e: anyhow::Error| (Reply::Error { message: e.to_string() }, None);
    if matches!(request, Request::Key { .. } | Request::Tap { .. } | Request::Swipe { .. } | Request::Slide { .. }) {
        wake(weak, actions).await;
    }
    match request {
        Request::Key { key: k, action } => {
            key(weak, k, action).await;
            (Reply::Ok, None)
        }
        Request::Tap { at } => {
            swipe(weak, at, at, TAP.as_millis() as u32).await;
            (Reply::Ok, None)
        }
        Request::Swipe { from, to, ms } => {
            swipe(weak, from, to, ms).await;
            (Reply::Ok, None)
        }
        Request::Slide { steps } => {
            let _ = actions.send(UiAction::Slider(Gesture::Steps(steps)));
            (Reply::Ok, None)
        }
        Request::Page { name } => match page_by_name(&name) {
            Some(page) => match on_ui(weak, move |app| app.go_to(page, 0)).await {
                Ok(()) => (Reply::Ok, None),
                Err(e) => error(e),
            },
            None => (Reply::Error { message: format!("no page {name:?}") }, None),
        },
        Request::State => match on_ui(weak, |app| state(&app)).await {
            Ok(s) => (Reply::State(s), None),
            Err(e) => error(e),
        },
        Request::Screenshot => match screenshot(weak).await {
            Ok((reply, png)) => (reply, Some(png)),
            Err(e) => error(e),
        },
    }
}
