//! The poster wall: while Kodi shows its Movies or TV list and nothing plays, the remote mirrors
//! that list with the highlighted title lifted out. Kodi sends no event when the highlight
//! moves, so it is asked after every key press, and once a second besides.

use crate::{integrations::kodi, AppWindow, LibItem};
use serde_json::Value;
use slint::Model as _;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::{watch, Notify};

const W: u32 = 136;
const H: u32 = 204;
/// Posters fetched either side of the highlight.
const AHEAD: usize = 15;

/// Kodi's web address while the wall may show; `None` otherwise.
static GATE: OnceLock<watch::Sender<Option<String>>> = OnceLock::new();
static POKE: OnceLock<Arc<Notify>> = OnceLock::new();
/// Bumped for each new list, so posters for an old one are dropped.
static LIST: AtomicU64 = AtomicU64::new(0);
static CURSOR: Mutex<Cursor> = Mutex::new(Cursor { at: 0, len: 0, column: -1, columns: 0, held: None, vertical: None });
/// How long a predicted move stands against Kodi's answers, which may predate the key press.
const HOLD: Duration = Duration::from_millis(400);

/// Kodi's highlight as last reported or predicted, so a d-pad press moves the wall before Kodi answers.
struct Cursor {
    at: usize,
    /// Titles on the wall; 0 while it is not showing.
    len: usize,
    column: i64,
    /// Kodi's posters per row, once a report has shown it.
    columns: i64,
    held: Option<Instant>,
    /// Where an up or down press left from before the row width was known; the move it makes gives the width.
    vertical: Option<usize>,
}

/// For recordings: titles named in `UC_DEMO_HIDE` (separated by `|`) are left off the wall, to put a film in
/// the column a take needs. Kodi's own list is untouched.
fn demo_hidden(it: &Value) -> bool {
    static HIDE: OnceLock<Vec<String>> = OnceLock::new();
    let hide = HIDE.get_or_init(|| {
        std::env::var("UC_DEMO_HIDE").unwrap_or_default().split('|').map(str::trim).filter(|t| !t.is_empty()).map(String::from).collect()
    });
    !hide.is_empty() && it["label"].as_str().is_some_and(|l| hide.iter().any(|h| h == l))
}

/// A d-pad key went to Kodi: move the wall's highlight where Kodi's will go. Kodi's panel stops at the end
/// of a row and moves a whole row of its own up or down.
pub fn predict(weak: &slint::Weak<AppWindow>, cmd: &str) {
    let mut c = CURSOR.lock().unwrap_or_else(|e| e.into_inner());
    let (at, len, col, cols) = (c.at as i64, c.len as i64, c.column, c.columns);
    if len == 0 || col < 0 {
        return;
    }
    let (at, column) = match cmd {
        "cursor_right" if at + 1 < len && (cols == 0 || col + 1 < cols) => (at + 1, col + 1),
        "cursor_left" if at > 0 && col > 0 => (at - 1, col - 1),
        "cursor_down" if cols > 0 && at + cols < len => (at + cols, col),
        "cursor_up" if cols > 0 && at - cols >= 0 => (at - cols, col),
        "cursor_up" | "cursor_down" if cols == 0 => {
            c.vertical = Some(c.at);
            return;
        }
        _ => return,
    };
    c.at = at as usize;
    c.column = column;
    c.held = Some(Instant::now() + HOLD);
    crate::view::ui_call(weak, move |app| {
        if app.get_lib_mode() == 4 {
            app.set_lib_focus(at as i32);
        }
    });
}

/// Called with every model change: whether the wall may show at all.
pub fn allow(yes: bool) {
    let want = if yes { kodi::endpoint() } else { None };
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
    let mpaa = certificate(it["mpaa"].as_str().unwrap_or(""));
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

/// Kodi's certification ("Rated PG-13", "Not Rated", "TV-MA") as the code the badges are named by.
fn certificate(raw: &str) -> String {
    let code = raw.trim().trim_start_matches("Rated ").trim().to_uppercase();
    match code.as_str() {
        "NOT RATED" | "UNRATED" | "NR" => "NR".into(),
        _ => code,
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
                        let list: Vec<Value> = list.into_iter().filter(|it| !demo_hidden(it)).collect();
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
                        crate::view::ui_call(&weak, move |app| {
                            let rows: Vec<LibItem> = texts.iter().map(|e| item(e, slint::Image::default())).collect();
                            app.set_lib_items(std::rc::Rc::new(slint::VecModel::from(rows)).into());
                            app.set_lib_wall_style(crate::app::settings::get().wall_style.unwrap_or(2));
                            app.set_lib_heading(heading.into());
                            app.set_lib_count(count.into());
                            URLS.with(|u| *u.borrow_mut() = (generation, urls));
                            // A different list while the wall is up comes in the same way.
                            if app.get_lib_mode() == 4 {
                                crate::view::stage::reveal_library(&app, std::time::Duration::ZERO);
                            }
                        });
                    }
                    let ids = &shown.as_ref().unwrap().1;
                    let at = ids.iter().position(|&id| id == view.focus).unwrap_or(0);
                    let stale = {
                        let mut c = CURSOR.lock().unwrap_or_else(|e| e.into_inner());
                        c.len = ids.len();
                        // Kodi's answer may be from before the last key press: while a prediction stands, one
                        // that disagrees is asked again shortly rather than shown.
                        c.held.is_some_and(|t| Instant::now() < t) && at != c.at
                    };
                    if stale {
                        tokio::time::sleep(Duration::from_millis(120)).await;
                        continue;
                    }
                    {
                        let mut c = CURSOR.lock().unwrap_or_else(|e| e.into_inner());
                        c.held = None;
                        c.at = at;
                        c.column = view.column;
                        if view.row > 0 && view.column >= 0 && view.position >= view.column {
                            c.columns = (view.position - view.column) / view.row;
                        } else if let Some(from) = c.vertical.filter(|&f| f != at) {
                            c.columns = from.abs_diff(at) as i64;
                        }
                        if c.columns > 0 {
                            c.vertical = None;
                        }
                    }
                    crate::view::ui_call(&weak, move |app| {
                        app.set_lib_focus(at as i32);
                        crate::view::stage::library_stage(&app, 4);
                    });
                    fetch_posters(&weak, ids, at, &loading);
                }
                _ => {
                    CURSOR.lock().unwrap_or_else(|e| e.into_inner()).len = 0;
                    if shown.take().is_some() {
                        crate::view::ui_call(&weak, |app| crate::view::stage::library_stage(&app, 0));
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
    crate::view::ui_call(&weak.clone(), move |_| {
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
                let img = match crate::art::fetch_image(&url) {
                    Ok(img) => poster(&img),
                    Err(e) => {
                        tracing::debug!("library poster {url}: {e:#}");
                        continue;
                    }
                };
                crate::view::ui_call(&weak, move |app| {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn certificates_match_the_badge_names() {
        assert_eq!(certificate("Rated PG-13"), "PG-13");
        assert_eq!(certificate("tv-ma"), "TV-MA");
        assert_eq!(certificate("Not Rated"), "NR");
        assert_eq!(certificate(""), "");
    }
}
