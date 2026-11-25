//! What Kodi's GUI shows: InfoLabels (free strings, not in the schema) and `GUI.GetProperties`.

use crate::rpc::Method;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The InfoLabels the client reads. They aren't in the JSON-RPC schema, so this list is the one place they live.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InfoLabel {
    ContainerContent,
    ContainerFolderPath,
    ContainerSortMethod,
    ContainerSortOrder,
    ContainerNumItems,
    ContainerCurrentItem,
    ContainerPosition,
    ContainerColumn,
    ContainerRow,
    ListItemDbId,
    ListItemDbType,
    PlayerChapter,
    PlayerChapterCount,
    PlayerChapterName,
}

impl InfoLabel {
    pub fn name(self) -> &'static str {
        use InfoLabel::*;
        match self {
            ContainerContent => "Container.Content",
            ContainerFolderPath => "Container.FolderPath",
            ContainerSortMethod => "Container.SortMethod",
            ContainerSortOrder => "Container.SortOrder",
            ContainerNumItems => "Container.NumItems",
            ContainerCurrentItem => "Container.CurrentItem",
            ContainerPosition => "Container.Position",
            ContainerColumn => "Container.Column",
            ContainerRow => "Container.Row",
            ListItemDbId => "ListItem.DBID",
            ListItemDbType => "ListItem.DBTYPE",
            PlayerChapter => "Player.Chapter",
            PlayerChapterCount => "Player.ChapterCount",
            PlayerChapterName => "Player.ChapterName",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GetInfoLabels {
    pub labels: Vec<&'static str>,
}

impl GetInfoLabels {
    pub fn of(labels: &[InfoLabel]) -> Self {
        Self { labels: labels.iter().map(|l| l.name()).collect() }
    }
}

impl Method for GetInfoLabels {
    const NAME: &'static str = "XBMC.GetInfoLabels";
    type Output = InfoLabels;
}

/// The answers, by label.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(transparent)]
pub struct InfoLabels(pub HashMap<String, String>);

impl InfoLabels {
    pub fn text(&self, l: InfoLabel) -> &str {
        self.0.get(l.name()).map_or("", String::as_str)
    }

    pub fn number(&self, l: InfoLabel) -> Option<i64> {
        self.text(l).trim().parse().ok()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GetGuiProperties {
    pub properties: &'static [GuiProperty],
}

/// The subset of `GUI.Property.Name` read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GuiProperty {
    CurrentWindow,
    Fullscreen,
}

impl Method for GetGuiProperties {
    const NAME: &'static str = "GUI.GetProperties";
    type Output = GuiProps;
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct GuiProps {
    pub currentwindow: Option<Window>,
    pub fullscreen: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Window {
    pub id: i32,
    pub label: String,
}
