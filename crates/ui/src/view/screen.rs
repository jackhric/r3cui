//! Names for the numbers the window's state is kept in. Slint holds pages, library modes and the join state as
//! ints (see the comments by their properties in app.slint); Rust only ever reads and writes them through these.

use crate::AppWindow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page {
    Home = 0,
    Inputs = 1,
    Menu = 2,
    Display = 3,
    Power = 4,
    Wifi = 5,
    Password = 6,
    Status = 7,
    JoinFailed = 8,
    Tracks = 9,
    FeedbackLab = 10,
    Integrations = 11,
}

impl Page {
    /// From the number app.slint uses, as given to `--page`.
    pub(crate) fn from_number(n: i32) -> Option<Self> {
        use Page::*;
        [Home, Inputs, Menu, Display, Power, Wifi, Password, Status, JoinFailed, Tracks, FeedbackLab, Integrations].into_iter().find(|p| *p as i32 == n)
    }
}

/// What the stage shows of Kodi's library. Only `Off` and `Wall` are live; the rest are the previews' variations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LibMode {
    Off = 0,
    River = 1,
    RiverCard = 2,
    Carousel = 3,
    Wall = 4,
    DimRiverCard = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JoinState {
    Idle = 0,
    Joining = 1,
    Failed = 2,
}

/// The audio or the subtitle picker (Slint's `picker`, 0 or 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Picker {
    Audio = 0,
    Subtitles = 1,
}

impl Picker {
    pub(crate) fn from_slint(kind: i32) -> Self {
        if kind == 0 { Self::Audio } else { Self::Subtitles }
    }
}

/// Which picture a blur is taken from (Slint's `capturing`): the stage alone, or the stage with the world over it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Capture {
    Stage = 1,
    World = 2,
}

impl Capture {
    /// Slint's `capture(which)` callback: 0 the stage, 1 the world.
    pub(crate) fn from_slint(which: i32) -> Self {
        if which == 0 { Self::Stage } else { Self::World }
    }
}

/// The window's state, by name.
pub(crate) trait Screen {
    fn on_page(&self, page: Page) -> bool;
    fn go_to(&self, page: Page, focus: i32);
    fn lib_mode(&self) -> LibMode;
    fn show_lib_mode(&self, mode: LibMode);
    fn show_join(&self, state: JoinState);
    #[cfg(feature = "preview")]
    fn show_picker(&self, picker: Picker);
}

impl Screen for AppWindow {
    fn on_page(&self, page: Page) -> bool {
        self.get_page() == page as i32
    }
    fn go_to(&self, page: Page, focus: i32) {
        self.invoke_go(page as i32, focus);
    }
    fn lib_mode(&self) -> LibMode {
        match self.get_lib_mode() {
            1 => LibMode::River,
            2 => LibMode::RiverCard,
            3 => LibMode::Carousel,
            4 => LibMode::Wall,
            5 => LibMode::DimRiverCard,
            _ => LibMode::Off,
        }
    }
    fn show_lib_mode(&self, mode: LibMode) {
        self.set_lib_mode(mode as i32);
    }
    fn show_join(&self, state: JoinState) {
        self.set_join_state(state as i32);
    }
    #[cfg(feature = "preview")]
    fn show_picker(&self, picker: Picker) {
        self.set_picker(picker as i32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_by_number() {
        assert_eq!(Page::from_number(9), Some(Page::Tracks));
        assert_eq!(Page::from_number(10), Some(Page::FeedbackLab));
        assert_eq!(Page::from_number(11), Some(Page::Integrations));
        assert_eq!(Page::from_number(12), None);
    }
}
