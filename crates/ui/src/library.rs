//! The poster wall: while Kodi shows its Movies or TV list and nothing plays, the remote mirrors
//! that list with the highlighted title lifted out. Kodi sends no event when the highlight
//! moves, so it is asked after every key press, and once a second besides.

use crate::{kodi, AppWindow, LibItem};
use serde_json::Value;
use slint::Model as _;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::{watch, Notify};

const W: u32 = 136;
const H: u32 = 204;
/// Posters fetched either side of the highlight.
const AHEAD: usize = 15;

/// Kodi's web address while the wall may show; `None` otherwise.
static GATE: OnceLock<watch::Sender<Option<String>>> = OnceLock::new();
static POKE: OnceLock<Arc<Notify>> = OnceLock::new();
static BASE: OnceLock<String> = OnceLock::new();
/// Bumped for each new list, so posters for an old one are dropped.
static LIST: AtomicU64 = AtomicU64::new(0);

/// `kodi_driver.main.media_player.<host>` → `http://<host>:8080`: the Kodi driver names its
/// entities after the host, and the remote already reaches Kodi's artwork there.
pub fn set_kodi(screen_entity: &str) {
    let host = screen_entity.rsplit_once("media_player.").map(|(_, h)| h).unwrap_or("");
    if screen_entity.starts_with("kodi") && !host.is_empty() {
        let _ = BASE.set(format!("http://{host}:8080"));
    }
}

/// Called with every model change: whether the wall may show at all.
pub fn allow(yes: bool) {
    let want = if yes { BASE.get().cloned() } else { None };
    if let Some(gate) = GATE.get() {
        gate.send_if_modified(|v| {
            let changed = *v != want;
            *v = want;
            changed
        });
    }
}

/// A key went to Kodi: look again shortly.
pub fn poke() {
    if let Some(p) = POKE.get() {
        p.notify_one();
    }
}

pub struct Entry {
    pub id: i64,
    pub title: String,
    pub meta: String,
    pub rating: String,
    pub tail: String,
    pub genre: String,
    pub plot: String,
    pub detail: String,
    pub progress: f32,
}

pub fn entry(it: &Value, shows: bool) -> Entry {
    let year = it["year"].as_i64().unwrap_or(0);
    let rating = it["rating"].as_f64().unwrap_or(0.0);
    let genre: Vec<&str> = it["genre"].as_array().map(|g| g.iter().filter_map(Value::as_str).take(3).collect()).unwrap_or_default();
    let mpaa = it["mpaa"].as_str().unwrap_or("").trim_start_matches("Rated ").to_string();
    let (meta, detail, progress) = if shows {
        let (eps, seen) = (it["episode"].as_i64().unwrap_or(0), it["watchedepisodes"].as_i64().unwrap_or(0));
        let seasons = it["season"].as_i64().unwrap_or(0);
        let meta = format!("{year} · {seasons} season{}", if seasons == 1 { "" } else { "s" });
        (meta, format!("{seen} of {eps} episodes watched"), if eps > 0 { seen as f32 / eps as f32 } else { 0.0 })
    } else {
        let mins = it["runtime"].as_i64().unwrap_or(0) / 60;
        let meta = format!("{year} · {}h {:02}m", mins / 60, mins % 60);
        let (pos, total) = (it["resume"]["position"].as_f64().unwrap_or(0.0), it["resume"]["total"].as_f64().unwrap_or(0.0));
        let detail = if pos > 0.0 {
            format!("Resume from {}:{:02}", pos as i64 / 3600, pos as i64 / 60 % 60)
        } else if it["playcount"].as_i64().unwrap_or(0) > 0 {
            "Watched".into()
        } else {
            String::new()
        };
        (meta, detail, if total > 0.0 { (pos / total) as f32 } else { 0.0 })
    };
    Entry {
        id: it["id"].as_i64().unwrap_or(-1),
        title: it["title"].as_str().or(it["label"].as_str()).unwrap_or("").into(),
        meta,
        rating: if rating > 0.0 { format!("{rating:.1}") } else { String::new() },
        tail: mpaa,
        genre: genre.join(" · "),
        plot: it["plot"].as_str().unwrap_or("").into(),
        detail,
        progress,
    }
}

pub fn item(e: &Entry, poster: slint::Image) -> LibItem {
    LibItem {
        poster,
        title: e.title.as_str().into(),
        meta: e.meta.as_str().into(),
        rating: e.rating.as_str().into(),
        tail: e.tail.as_str().into(),
        genre: e.genre.as_str().into(),
        plot: e.plot.as_str().into(),
        detail: e.detail.as_str().into(),
        progress: e.progress,
    }
}

pub fn poster(img: &image::DynamicImage) -> slint::SharedPixelBuffer<slint::Rgb8Pixel> {
    let small = img.resize_to_fill(W, H, image::imageops::FilterType::Triangle).to_rgb8();
    slint::SharedPixelBuffer::clone_from_slice(small.as_raw(), W, H)
}

pub fn start(weak: slint::Weak<AppWindow>) {
    let (tx, mut gate) = watch::channel(None::<String>);
    let _ = GATE.set(tx);
    let poke = POKE.get_or_init(|| Arc::new(Notify::new())).clone();
    tokio::spawn(async move {
        // (folder, sort, order) of the list on show, and its ids in order.
        let mut shown: Option<(String, Vec<i64>)> = None;
        let loading: Arc<Mutex<HashSet<i64>>> = Arc::default();
        loop {
            let base = gate.borrow_and_update().clone();
            let view = match base.clone() {
                Some(b) => tokio::task::spawn_blocking(move || kodi::browsing(&b)).await.ok().and_then(|r| r.map_err(|e| tracing::debug!("kodi browsing: {e:#}")).ok()),
                None => None,
            };
            match (base, view) {
                (Some(base), Some(view)) if matches!(view.content.as_str(), "movies" | "tvshows") => {
                    let key = format!("{}|{}|{}", view.path, view.sort, view.order);
                    if shown.as_ref().is_none_or(|(k, _)| *k != key) {
                        let (b, v) = (base.clone(), view.clone());
                        let Ok(Ok(list)) = tokio::task::spawn_blocking(move || kodi::list(&b, &v)).await else {
                            tracing::warn!("kodi list {} failed", view.path);
                            wait(&mut gate, &poke).await;
                            continue;
                        };
                        let shows = view.content == "tvshows";
                        let entries: Vec<Entry> = list.iter().map(|it| entry(it, shows)).collect();
                        let urls: Vec<String> = list.iter().map(|it| it["poster"].as_str().unwrap_or("").to_string()).collect();
                        tracing::info!("library wall: {} {} by {} {}", entries.len(), view.content, view.sort, view.order);
                        let generation = LIST.fetch_add(1, Ordering::SeqCst) + 1;
                        loading.lock().unwrap().clear();
                        shown = Some((key, entries.iter().map(|e| e.id).collect()));
                        let heading = if shows { "TV SHOWS" } else { "MOVIES" };
                        let count = format!("{} {}", entries.len(), if shows { "shows" } else { "films" });
                        let texts: Vec<_> = entries;
                        crate::ui_call(&weak, move |app| {
                            let rows: Vec<LibItem> = texts.iter().map(|e| item(e, slint::Image::default())).collect();
                            app.set_lib_items(std::rc::Rc::new(slint::VecModel::from(rows)).into());
                            app.set_lib_heading(heading.into());
                            app.set_lib_count(count.into());
                            URLS.with(|u| *u.borrow_mut() = (generation, urls));
                            // A different list while the wall is up comes in the same way.
                            if app.get_lib_mode() == 4 {
                                crate::reveal_library(&app, std::time::Duration::ZERO);
                            }
                        });
                    }
                    let ids = &shown.as_ref().unwrap().1;
                    let at = ids.iter().position(|&id| id == view.focus).unwrap_or(0);
                    crate::ui_call(&weak, move |app| {
                        app.set_lib_focus(at as i32);
                        crate::library_stage(&app, 4);
                    });
                    fetch_posters(&weak, ids, at, &loading);
                }
                _ => {
                    if shown.take().is_some() {
                        crate::ui_call(&weak, |app| crate::library_stage(&app, 0));
                    }
                }
            }
            wait(&mut gate, &poke).await;
        }
    });
}

thread_local! {
    /// The UI thread's copy of the poster URLs for the list on show, with its generation.
    static URLS: std::cell::RefCell<(u64, Vec<String>)> = std::cell::RefCell::default();
}

async fn wait(gate: &mut watch::Receiver<Option<String>>, poke: &Notify) {
    tokio::select! {
        _ = poke.notified() => tokio::time::sleep(Duration::from_millis(90)).await,
        _ = tokio::time::sleep(Duration::from_secs(1)) => {}
        _ = gate.changed() => {}
    }
}

fn fetch_posters(weak: &slint::Weak<AppWindow>, ids: &[i64], at: usize, loading: &Arc<Mutex<HashSet<i64>>>) {
    let lo = at.saturating_sub(AHEAD);
    let hi = (at + AHEAD + 1).min(ids.len());
    // Nearest first, so the highlighted poster lands before the far ones.
    let mut want: Vec<usize> = (lo..hi).filter(|i| !loading.lock().unwrap().contains(&ids[*i])).collect();
    want.sort_by_key(|i| i.abs_diff(at));
    if want.is_empty() {
        return;
    }
    loading.lock().unwrap().extend(want.iter().map(|&i| ids[i]));
    let generation = LIST.load(Ordering::SeqCst);
    let weak = weak.clone();
    // The UI thread holds the URLs; ask it for the ones wanted, then fetch off it.
    crate::ui_call(&weak.clone(), move |_| {
        let urls: Vec<(usize, String)> = URLS.with(|u| {
            let u = u.borrow();
            if u.0 != generation {
                return Vec::new();
            }
            want.iter().filter_map(|&i| Some((i, u.1.get(i)?.clone()))).filter(|(_, url)| !url.is_empty()).collect()
        });
        std::thread::spawn(move || {
            for (i, url) in urls {
                if LIST.load(Ordering::SeqCst) != generation {
                    return;
                }
                let img = match crate::fetch_image(&url) {
                    Ok(img) => poster(&img),
                    Err(e) => {
                        tracing::debug!("library poster {url}: {e:#}");
                        continue;
                    }
                };
                crate::ui_call(&weak, move |app| {
                    if LIST.load(Ordering::SeqCst) != generation {
                        return;
                    }
                    let items = app.get_lib_items();
                    if let Some(mut row) = items.row_data(i) {
                        row.poster = slint::Image::from_rgb8(img);
                        items.set_row_data(i, row);
                    }
                });
            }
        });
    });
}
