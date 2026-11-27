use crate::{AppWindow, Lib};
use slint::{ComponentHandle, ModelRc};
use std::path::PathBuf;
use std::rc::Rc;

/// Kodi's library as saved by a fetch script: `library.json` plus a poster file per title, in
/// the directory named by `UC_DEMO_LIBRARY`.
pub(super) struct Shelf {
    pub(super) posters: Vec<slint::Image>,
    pub(super) items: Vec<crate::LibItem>,
    pub(super) count: String,
}

impl Shelf {
    pub(super) fn items(&self) -> ModelRc<crate::LibItem> {
        Rc::new(slint::VecModel::from(self.items.clone())).into()
    }

    /// Put the titles on the library page under `heading`.
    pub(super) fn list(&self, app: &AppWindow, heading: &str) {
        app.global::<Lib>().set_heading(heading.into());
        app.global::<Lib>().set_count(self.count.clone().into());
        app.global::<Lib>().set_items(self.items());
    }

    /// The first title containing `UC_DEMO_FOCUS`, else the first.
    pub(super) fn focus(&self) -> i32 {
        let want = std::env::var("UC_DEMO_FOCUS").unwrap_or_default().to_lowercase();
        self.items.iter().position(|i| !want.is_empty() && i.title.to_lowercase().contains(&want)).unwrap_or(0) as i32
    }
}

pub(super) fn shelf(kind: &str) -> Shelf {
    use serde_json::Value;
    let dir = PathBuf::from(std::env::var("UC_DEMO_LIBRARY").unwrap_or_default());
    let json: Value = std::fs::read(dir.join("library.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let list = json[kind].as_array().cloned().unwrap_or_default();
    let shows = kind == "tvshows";
    let mut posters = Vec::new();
    let mut items = Vec::new();
    for it in &list {
        let Some(file) = it["file"].as_str() else { continue };
        // A missing file stands for a poster still on its way.
        let poster = match image::open(dir.join(file)) {
            Ok(img) => {
                let img = img.resize_to_fill(200, 300, image::imageops::FilterType::Triangle).to_rgb8();
                let poster = slint::Image::from_rgb8(slint::SharedPixelBuffer::clone_from_slice(img.as_raw(), 200, 300));
                posters.push(poster.clone());
                poster
            }
            Err(_) => slint::Image::default(),
        };
        let item: ::kodi::methods::library::FileItem = serde_json::from_value(it.clone()).unwrap_or_default();
        items.push(crate::view::library::item(&kodi::driver::library_item(&item, shows, String::new()), poster));
    }
    let count = if shows { format!("{} shows", list.len()) } else { format!("{} films", list.len()) };
    Shelf { posters, items, count }
}
