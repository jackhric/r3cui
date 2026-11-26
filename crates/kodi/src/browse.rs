//! What Kodi's GUI is browsing, for the remote's poster wall: the container's content, folder and sort, and where the
//! highlight is, read from InfoLabels; and the same list fetched in the same order.

use crate::methods::gui::{InfoLabel, InfoLabels};
use crate::methods::library::{Sort, SortMethod, SortOrder};

/// The labels one browse poll reads.
pub const LABELS: &[InfoLabel] = &[
    InfoLabel::ContainerContent,
    InfoLabel::ContainerFolderPath,
    InfoLabel::ContainerSortMethod,
    InfoLabel::ContainerSortOrder,
    InfoLabel::ListItemDbId,
    InfoLabel::ContainerColumn,
    InfoLabel::ContainerRow,
    InfoLabel::ContainerPosition,
];

/// `Container.Content` for the lists the client browses.
pub(crate) const MOVIES: &str = "movies";
const TV_SHOWS: &str = "tvshows";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    Movies,
    TvShows,
    /// Anything else, including nothing (outside a library list).
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Browsing {
    pub content: Content,
    pub path: String,
    /// The GUI's sort as it names it (translated), and its order.
    pub sort_label: String,
    pub ascending: bool,
    /// The highlighted item's library id.
    pub focus: Option<u32>,
    /// The highlight's column and row in Kodi's panel, and its place among the cells on screen; `None` when Kodi
    /// gives none.
    pub column: Option<i64>,
    pub row: Option<i64>,
    pub position: Option<i64>,
}

impl Browsing {
    pub fn from_labels(l: &InfoLabels) -> Self {
        let content = match l.text(InfoLabel::ContainerContent) {
            MOVIES => Content::Movies,
            TV_SHOWS => Content::TvShows,
            other => Content::Other(other.to_string()),
        };
        Self {
            content,
            path: l.text(InfoLabel::ContainerFolderPath).to_string(),
            sort_label: l.text(InfoLabel::ContainerSortMethod).to_string(),
            ascending: !l.text(InfoLabel::ContainerSortOrder).eq_ignore_ascii_case("descending"),
            focus: l.number(InfoLabel::ListItemDbId).and_then(|n| u32::try_from(n).ok()),
            column: l.number(InfoLabel::ContainerColumn),
            row: l.number(InfoLabel::ContainerRow),
            position: l.number(InfoLabel::ContainerPosition),
        }
    }

    /// A Movies or TV list the wall can mirror.
    pub fn is_library(&self) -> bool {
        matches!(self.content, Content::Movies | Content::TvShows)
    }

    /// The list's identity: a different folder or sort is a different list.
    pub fn list_key(&self) -> String {
        format!("{}|{}|{}", self.path, self.sort_label, self.ascending)
    }

    /// The GUI's sort as `List.Sort`. Kodi names the sort in its own language; these are the English names, and
    /// anything else sorts by title, as the GUI's Title sort does: by each item's sort title ("bronx tale").
    pub fn sort(&self) -> Sort {
        let method = match self.sort_label.to_lowercase().as_str() {
            "year" => SortMethod::Year,
            "date added" => SortMethod::DateAdded,
            "rating" => SortMethod::Rating,
            "user rating" => SortMethod::UserRating,
            "last played" => SortMethod::LastPlayed,
            "play count" => SortMethod::PlayCount,
            "duration" | "runtime" => SortMethod::Duration,
            "mpaa" => SortMethod::Mpaa,
            _ => SortMethod::SortTitle,
        };
        Sort { method, order: if self.ascending { SortOrder::Ascending } else { SortOrder::Descending }, ignorearticle: true }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn labels(pairs: &[(InfoLabel, &str)]) -> InfoLabels {
        InfoLabels(pairs.iter().map(|(l, v)| (l.name().to_string(), v.to_string())).collect::<HashMap<_, _>>())
    }

    #[test]
    fn reads_the_gui() {
        use InfoLabel::*;
        let b = Browsing::from_labels(&labels(&[
            (ContainerContent, "movies"),
            (ContainerFolderPath, "videodb://movies/titles/"),
            (ContainerSortMethod, "Duration"),
            (ContainerSortOrder, "Descending"),
            (ListItemDbId, "39"),
            (ContainerColumn, "4"),
            (ContainerRow, "1"),
            (ContainerPosition, "10"),
        ]));
        assert!(b.is_library());
        assert_eq!((b.focus, b.column, b.row, b.position), (Some(39), Some(4), Some(1), Some(10)));
        assert_eq!(b.sort(), Sort { method: SortMethod::Duration, order: SortOrder::Descending, ignorearticle: true });
        let home = Browsing::from_labels(&labels(&[(ContainerContent, ""), (ListItemDbId, "")]));
        assert!(!home.is_library() && home.focus.is_none());
        assert_eq!(home.sort().method, SortMethod::SortTitle);
    }
}
