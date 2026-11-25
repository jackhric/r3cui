//! The library as Kodi's GUI lists it: `Files.GetDirectory` on the folder the GUI shows, sorted as the GUI sorts.

use crate::rpc::Method;
use crate::wire::Art;
use serde::{Deserialize, Serialize};

/// The fields the wall reads, a subset of `List.Fields.Files`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FileField {
    Title,
    Year,
    Art,
    Runtime,
    Rating,
    Genre,
    Plot,
    Resume,
    PlayCount,
    Mpaa,
    Season,
    Episode,
    WatchedEpisodes,
}

pub const MOVIE_FIELDS: &[FileField] = {
    use FileField::*;
    &[Title, Year, Art, Runtime, Rating, Genre, Plot, Resume, PlayCount, Mpaa]
};
pub const SHOW_FIELDS: &[FileField] = {
    use FileField::*;
    &[Title, Year, Art, Rating, Genre, Plot, Season, Episode, WatchedEpisodes, Mpaa]
};

#[derive(Debug, Clone, Serialize)]
pub struct GetDirectory {
    pub directory: String,
    pub media: &'static str,
    pub properties: &'static [FileField],
    pub sort: Sort,
}
impl Method for GetDirectory {
    const NAME: &'static str = "Files.GetDirectory";
    type Output = Directory;
}

/// `List.Sort`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Sort {
    pub method: SortMethod,
    pub order: SortOrder,
    pub ignorearticle: bool,
}

/// The subset of `List.Sort.method` the GUI's sorts map to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SortMethod {
    SortTitle,
    Title,
    Year,
    DateAdded,
    Rating,
    UserRating,
    LastPlayed,
    PlayCount,
    /// The GUI's "Duration" sort; Kodi calls it `time`.
    #[serde(rename = "time")]
    Duration,
    Mpaa,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    Ascending,
    Descending,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Directory {
    #[serde(default)]
    pub files: Vec<FileItem>,
}

/// One entry of `List.Item.File`, the fields asked for.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct FileItem {
    #[serde(default)]
    pub id: Option<u32>,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub year: i32,
    #[serde(default)]
    pub art: Art,
    #[serde(default)]
    pub runtime: u32,
    #[serde(default)]
    pub rating: f64,
    #[serde(default)]
    pub genre: Vec<String>,
    #[serde(default)]
    pub plot: String,
    #[serde(default)]
    pub resume: Resume,
    #[serde(default)]
    pub playcount: u32,
    #[serde(default)]
    pub mpaa: String,
    #[serde(default)]
    pub season: i32,
    #[serde(default)]
    pub episode: i32,
    #[serde(default)]
    pub watchedepisodes: i32,
}

/// `Video.Resume`, in seconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
pub struct Resume {
    #[serde(default)]
    pub position: f64,
    #[serde(default)]
    pub total: f64,
}
