//! The library wall on screen: the list a UI driver publishes, with posters fetched around the highlight, nearest
//! first, as it moves.

use crate::view::stage::{library_stage, reveal_library};
use crate::view::ui;
use crate::{AppWindow, LibItem, LibMode, WallStyle};
use drivers::api::{Library, LibraryItem, LibraryList};
use slint::Model as _;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Posters fetched either side of the highlight.
const AHEAD: usize = 15;

pub(crate) struct Wall {
    weak: slint::Weak<AppWindow>,
    shown: Option<Library>,
    /// The list on show, so posters still loading for an earlier one are dropped.
    list: Arc<AtomicU64>,
    loading: Arc<Mutex<HashSet<usize>>>,
}

impl Wall {
    pub(crate) fn new(weak: slint::Weak<AppWindow>) -> Self {
        Self { weak, shown: None, list: Arc::default(), loading: Arc::default() }
    }

    pub(crate) fn show(&mut self, next: Option<&Library>) {
        if self.shown.as_ref() == next {
            return;
        }
        let Some(lib) = next else {
            self.shown = None;
            ui(&self.weak, |app| library_stage(&app, LibMode::Off));
            return;
        };
        if self.shown.as_ref().is_none_or(|s| s.list.id != lib.list.id) {
            self.list.store(lib.list.id, Ordering::SeqCst);
            self.loading.lock().unwrap().clear();
            show_list(&self.weak, lib.list.clone());
        }
        let at = lib.focus;
        ui(&self.weak, move |app| {
            app.set_lib_focus(at as i32);
            library_stage(&app, LibMode::Wall);
        });
        self.fetch_posters(&lib.list, at);
        self.shown = Some(lib.clone());
    }

    /// The posters around the highlight not yet fetched, nearest first so the highlighted one lands before the far
    /// ones. They come one after another on tokio's blocking pool, and stop if the list changes meanwhile.
    fn fetch_posters(&self, list: &Arc<LibraryList>, at: usize) {
        let (lo, hi) = (at.saturating_sub(AHEAD), (at + AHEAD + 1).min(list.items.len()));
        let mut want: Vec<usize> = {
            let mut loading = self.loading.lock().unwrap();
            (lo..hi).filter(|&i| !list.items[i].poster.is_empty() && loading.insert(i)).collect()
        };
        want.sort_by_key(|i| i.abs_diff(at));
        if want.is_empty() {
            return;
        }
        let (weak, current, list) = (self.weak.clone(), self.list.clone(), list.clone());
        tokio::task::spawn_blocking(move || {
            for i in want {
                if current.load(Ordering::SeqCst) != list.id {
                    return;
                }
                let url = &list.items[i].poster;
                let img = match crate::art::fetch_image(url) {
                    Ok(img) => crate::art::wall_poster_pixels(&img),
                    Err(e) => {
                        tracing::debug!("library poster {url}: {e:#}");
                        continue;
                    }
                };
                let (current, id) = (current.clone(), list.id);
                ui(&weak, move |app| {
                    if current.load(Ordering::SeqCst) != id {
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

/// A new list: its titles at once, the posters as they arrive.
fn show_list(weak: &slint::Weak<AppWindow>, list: Arc<LibraryList>) {
    ui(weak, move |app| {
        let rows: Vec<LibItem> = list.items.iter().map(|e| item(e, slint::Image::default())).collect();
        app.set_lib_items(std::rc::Rc::new(slint::VecModel::from(rows)).into());
        app.set_lib_wall_style(WallStyle::from_config(crate::app::settings::get().wall_style));
        app.set_lib_heading(list.heading.as_str().into());
        app.set_lib_count(list.count.as_str().into());
        // A different list while the wall is up comes in the same way.
        if app.get_lib_mode() == LibMode::Wall {
            reveal_library(&app, Duration::ZERO);
        }
    });
}

pub(crate) fn item(e: &LibraryItem, poster: slint::Image) -> LibItem {
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
