//! The poster wall: while Kodi shows its Movies or TV list and nothing plays, the remote mirrors that list with the
//! highlighted title lifted out. Kodi sends no event when the highlight moves, so it is asked after every key press,
//! and once a second besides.
//!
//! `Library` is the handle the session holds; one task behind it follows Kodi and fills the wall.

mod cursor;
pub(crate) mod entry;

use crate::input::KodiAction;
use crate::view::screen::{LibMode, Screen};
use ::kodi::methods::library::FileItem;
use ::kodi::KodiHandle;
use crate::{AppWindow, LibItem};
use cursor::{Cursor, Report};
use entry::{entry, item, poster, Entry};
use slint::Model as _;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{watch, Notify};

/// Posters fetched either side of the highlight.
const AHEAD: usize = 15;

#[derive(Clone)]
pub(crate) struct Library {
    shared: Arc<Shared>,
    weak: slint::Weak<AppWindow>,
}

struct Shared {
    /// Whether the wall may show.
    gate: watch::Sender<bool>,
    /// Kodi, once the session has one.
    kodi: Mutex<Option<KodiHandle>>,
    /// A key went to Kodi: look again shortly.
    poke: Notify,
    cursor: Mutex<Cursor>,
    /// Bumped for each new list, so posters still loading for an old one are dropped.
    list: AtomicU64,
    /// For recordings: titles left off the wall (see `UC_DEMO_HIDE`).
    hidden: Vec<String>,
}

impl Library {
    /// Start following Kodi; the wall shows only once `allow` says it may.
    pub(crate) fn start(weak: slint::Weak<AppWindow>) -> Self {
        let (gate, rx) = watch::channel(false);
        // Titles named in `UC_DEMO_HIDE` (separated by `|`) are left off the wall, to put a film in the column a take
        // needs. Kodi's own list is untouched.
        let hidden = std::env::var("UC_DEMO_HIDE").unwrap_or_default().split('|').map(str::trim).filter(|t| !t.is_empty()).map(String::from).collect();
        let shared = Arc::new(Shared { gate, kodi: Mutex::new(None), poke: Notify::new(), cursor: Mutex::default(), list: AtomicU64::new(0), hidden });
        let library = Self { shared, weak };
        tokio::spawn(library.clone().follow(rx));
        library
    }

    /// The Kodi to follow.
    pub(crate) fn attach(&self, kodi: KodiHandle) {
        *self.shared.kodi.lock().unwrap() = Some(kodi);
        self.shared.gate.send_modify(|_| {});
    }

    /// Whether the wall may show at all: Kodi on screen, nothing playing, the remote awake.
    pub(crate) fn allow(&self, yes: bool) {
        self.shared.gate.send_if_modified(|v| std::mem::replace(v, yes) != yes);
    }

    /// A key went to Kodi: move the wall's highlight where Kodi's will go, and look again shortly.
    pub(crate) fn pressed(&self, action: Option<KodiAction>) {
        if let Some(at) = action.and_then(|a| self.cursor().predict(a, Instant::now())) {
            crate::view::ui_call(&self.weak, move |app| {
                if app.lib_mode() == LibMode::Wall {
                    app.set_lib_focus(at as i32);
                }
            });
        }
        self.shared.poke.notify_one();
    }

    fn cursor(&self) -> std::sync::MutexGuard<'_, Cursor> {
        self.shared.cursor.lock().unwrap_or_else(|e| e.into_inner())
    }

    async fn follow(self, mut gate: watch::Receiver<bool>) {
        // (folder, sort, order) of the list on show, with its ids and poster URLs in order.
        let mut shown: Option<(String, Vec<i64>, Vec<String>)> = None;
        let loading: Arc<Mutex<HashSet<i64>>> = Arc::default();
        loop {
            let allowed = *gate.borrow_and_update();
            let kodi = self.shared.kodi.lock().unwrap().clone().filter(|_| allowed);
            let view = match &kodi {
                Some(k) => k.browsing().await.map_err(|e| tracing::debug!("kodi browsing: {e}")).ok(),
                None => None,
            };
            match (kodi, view) {
                (Some(kodi), Some(view)) if view.is_library() => {
                    let key = view.list_key();
                    if shown.as_ref().is_none_or(|(k, ..)| *k != key) {
                        let list = match kodi.listing(&view).await {
                            Ok(list) => list,
                            Err(e) => {
                                tracing::warn!("kodi list {}: {e}", view.path);
                                self.wait(&mut gate).await;
                                continue;
                            }
                        };
                        let list: Vec<FileItem> = list.into_iter().filter(|it| !self.hidden(it)).collect();
                        let shows = view.content == ::kodi::browse::Content::TvShows;
                        let entries: Vec<Entry> = list.iter().map(|it| entry(it, shows)).collect();
                        let urls = list
                            .iter()
                            .map(|it| it.art.poster.as_ref().or(it.art.thumb.as_ref()).map(|a| kodi.image_url(a, Some((136, 204)))).unwrap_or_default())
                            .collect();
                        tracing::info!("library wall: {} {:?} by {} {}", entries.len(), view.content, view.sort_label, if view.ascending { "ascending" } else { "descending" });
                        self.shared.list.fetch_add(1, Ordering::SeqCst);
                        loading.lock().unwrap().clear();
                        shown = Some((key, entries.iter().map(|e| e.id).collect(), urls));
                        self.show_list(entries, shows);
                    }
                    let (_, ids, urls) = shown.as_ref().unwrap();
                    let at = ids.iter().position(|&id| Some(id) == view.focus.map(i64::from)).unwrap_or(0);
                    let report = Report { at, len: ids.len(), column: view.column.unwrap_or(-1), row: view.row.unwrap_or(-1), position: view.position.unwrap_or(-1) };
                    if !self.cursor().report(report, Instant::now()) {
                        tokio::time::sleep(Duration::from_millis(120)).await;
                        continue;
                    }
                    crate::view::ui_call(&self.weak, move |app| {
                        app.set_lib_focus(at as i32);
                        crate::view::stage::library_stage(&app, LibMode::Wall);
                    });
                    self.fetch_posters(ids, urls, at, &loading);
                }
                _ => {
                    self.cursor().hide();
                    if shown.take().is_some() {
                        crate::view::ui_call(&self.weak, |app| crate::view::stage::library_stage(&app, LibMode::Off));
                    }
                }
            }
            self.wait(&mut gate).await;
        }
    }

    fn hidden(&self, it: &FileItem) -> bool {
        self.shared.hidden.iter().any(|h| *h == it.label)
    }

    /// A new list: its titles at once, the posters as they arrive.
    fn show_list(&self, entries: Vec<Entry>, shows: bool) {
        let heading = if shows { "TV SHOWS" } else { "MOVIES" };
        let count = format!("{} {}", entries.len(), if shows { "shows" } else { "films" });
        crate::view::ui_call(&self.weak, move |app| {
            let rows: Vec<LibItem> = entries.iter().map(|e| item(e, slint::Image::default())).collect();
            app.set_lib_items(std::rc::Rc::new(slint::VecModel::from(rows)).into());
            app.set_lib_wall_style(crate::app::settings::get().wall_style.unwrap_or(2));
            app.set_lib_heading(heading.into());
            app.set_lib_count(count.into());
            // A different list while the wall is up comes in the same way.
            if app.lib_mode() == LibMode::Wall {
                crate::view::stage::reveal_library(&app, Duration::ZERO);
            }
        });
    }

    async fn wait(&self, gate: &mut watch::Receiver<bool>) {
        tokio::select! {
            _ = self.shared.poke.notified() => tokio::time::sleep(Duration::from_millis(90)).await,
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            _ = gate.changed() => {}
        }
    }

    /// The posters around the highlight not yet fetched, nearest first so the highlighted one lands before the far
    /// ones. They come one after another on tokio's blocking pool, and stop if the list changes meanwhile.
    fn fetch_posters(&self, ids: &[i64], urls: &[String], at: usize, loading: &Arc<Mutex<HashSet<i64>>>) {
        let (lo, hi) = (at.saturating_sub(AHEAD), (at + AHEAD + 1).min(ids.len()));
        let mut want: Vec<usize> = {
            let mut loading = loading.lock().unwrap();
            (lo..hi).filter(|&i| loading.insert(ids[i])).collect()
        };
        want.sort_by_key(|i| i.abs_diff(at));
        let todo: Vec<(usize, String)> = want.into_iter().filter_map(|i| Some((i, urls.get(i)?.clone()))).filter(|(_, u)| !u.is_empty()).collect();
        if todo.is_empty() {
            return;
        }
        let (library, generation) = (self.clone(), self.shared.list.load(Ordering::SeqCst));
        tokio::task::spawn_blocking(move || {
            for (i, url) in todo {
                if library.shared.list.load(Ordering::SeqCst) != generation {
                    return;
                }
                let img = match crate::art::fetch_image(&url) {
                    Ok(img) => poster(&img),
                    Err(e) => {
                        tracing::debug!("library poster {url}: {e:#}");
                        continue;
                    }
                };
                let current = library.clone();
                crate::view::ui_call(&library.weak, move |app| {
                    if current.shared.list.load(Ordering::SeqCst) != generation {
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
    }
}
