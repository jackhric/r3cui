//! The library wall: while Kodi shows its Movies or TV list and nothing plays, the stage mirrors that list with the
//! highlighted title lifted out. Kodi sends no event when the highlight moves, so it is asked after every key press,
//! and once a second besides.

use super::cursor::{Cursor, Report};
use super::entry::entry;
use crate::api::{Library, LibraryList, RemoteKey};
use kodi::browse::Content;
use kodi::methods::library::FileItem;
use kodi::KodiHandle;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{watch, Notify};

/// The size the wall shows posters at, asked of Kodi's artwork source so it sends no more than that.
const POSTER: (u32, u32) = (136, 204);

#[derive(Clone)]
pub(crate) struct Wall {
    shared: Arc<Shared>,
}

struct Shared {
    kodi: KodiHandle,
    /// On screen, with the remote awake.
    shown: watch::Sender<bool>,
    view: watch::Sender<Option<Library>>,
    /// A key went to Kodi: look again shortly.
    poke: Notify,
    cursor: Mutex<Cursor>,
    /// For recordings: titles left off the wall (`UC_DEMO_HIDE`, separated by `|`), to put a film in the column a
    /// take needs. Kodi's own list is untouched.
    hidden: Vec<String>,
}

static LISTS: AtomicU64 = AtomicU64::new(1);

impl Wall {
    pub(crate) fn start(kodi: KodiHandle) -> (Self, watch::Receiver<Option<Library>>) {
        let (view, rx) = watch::channel(None);
        let hidden = std::env::var("UC_DEMO_HIDE").unwrap_or_default().split('|').map(str::trim).filter(|t| !t.is_empty()).map(String::from).collect();
        let shared = Arc::new(Shared { kodi, shown: watch::channel(false).0, view, poke: Notify::new(), cursor: Mutex::default(), hidden });
        let wall = Self { shared };
        tokio::spawn(wall.clone().follow());
        (wall, rx)
    }

    pub(crate) fn shown(&self, yes: bool) {
        self.shared.shown.send_if_modified(|v| std::mem::replace(v, yes) != yes);
    }

    /// A key went to Kodi: move the wall's highlight where Kodi's will go, and look again shortly.
    pub(crate) fn pressed(&self, key: RemoteKey) {
        if let Some(at) = self.cursor().predict(key, Instant::now()) {
            self.shared.view.send_if_modified(|v| match v {
                Some(lib) if lib.focus != at => {
                    lib.focus = at;
                    true
                }
                _ => false,
            });
        }
        self.shared.poke.notify_one();
    }

    fn cursor(&self) -> std::sync::MutexGuard<'_, Cursor> {
        self.shared.cursor.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn hide(&self) {
        self.cursor().hide();
        self.shared.view.send_if_modified(|v| v.take().is_some());
    }

    async fn follow(self) {
        let kodi = self.shared.kodi.clone();
        let (mut shown, mut playing) = (self.shared.shown.subscribe(), kodi.state());
        // The list on show: (Kodi's key for it, its ids in order, the published list).
        let mut list: Option<(String, Vec<i64>, Arc<LibraryList>)> = None;
        loop {
            let wanted = *shown.borrow_and_update() && playing.borrow_and_update().playback.is_none();
            let view = if wanted { kodi.browsing().await.map_err(|e| tracing::debug!("kodi browsing: {e}")).ok() } else { None };
            match view.filter(|v| v.is_library()) {
                Some(view) => {
                    let key = view.list_key();
                    if list.as_ref().is_none_or(|(k, ..)| *k != key) {
                        match kodi.listing(&view).await {
                            Ok(items) => {
                                let items: Vec<FileItem> = items.into_iter().filter(|it| !self.shared.hidden.contains(&it.label)).collect();
                                let shows = view.content == Content::TvShows;
                                tracing::info!("library wall: {} {:?} by {} {}", items.len(), view.content, view.sort_label, if view.ascending { "ascending" } else { "descending" });
                                list = Some((key, items.iter().map(|it| it.id.map_or(-1, i64::from)).collect(), Arc::new(self.list(&items, shows))));
                            }
                            Err(e) => tracing::warn!("kodi list {}: {e}", view.path),
                        }
                    }
                    if let Some((_, ids, items)) = &list {
                        let at = ids.iter().position(|&id| Some(id) == view.focus.map(i64::from)).unwrap_or(0);
                        let report = Report { at, len: ids.len(), column: view.column.unwrap_or(-1), row: view.row.unwrap_or(-1), position: view.position.unwrap_or(-1) };
                        if !self.cursor().report(report, Instant::now()) {
                            tokio::time::sleep(Duration::from_millis(120)).await;
                            continue;
                        }
                        let next = Library { list: items.clone(), focus: at };
                        self.shared.view.send_if_modified(|v| {
                            let changed = v.as_ref() != Some(&next);
                            *v = Some(next);
                            changed
                        });
                    }
                }
                None => {
                    list = None;
                    self.hide();
                }
            }
            tokio::select! {
                _ = self.shared.poke.notified() => tokio::time::sleep(Duration::from_millis(90)).await,
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                r = shown.changed() => if r.is_err() { break },
                r = playing.changed() => if r.is_err() { break },
            }
        }
    }

    fn list(&self, items: &[FileItem], shows: bool) -> LibraryList {
        let kodi = &self.shared.kodi;
        let poster = |it: &FileItem| it.art.poster.as_ref().or(it.art.thumb.as_ref()).map(|a| kodi.image_url(a, Some(POSTER))).unwrap_or_default();
        LibraryList {
            id: LISTS.fetch_add(1, Ordering::Relaxed),
            heading: if shows { "TV SHOWS" } else { "MOVIES" }.into(),
            count: format!("{} {}", items.len(), if shows { "shows" } else { "films" }),
            items: items.iter().map(|it| entry(it, shows, poster(it))).collect(),
        }
    }
}
