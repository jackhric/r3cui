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

/// The window's state, by name.
pub(crate) trait Screen {
    fn on_page(&self, page: Page) -> bool;
    fn go_to(&self, page: Page, focus: i32);
}

impl Screen for AppWindow {
    fn on_page(&self, page: Page) -> bool {
        self.get_page() == page as i32
    }
    fn go_to(&self, page: Page, focus: i32) {
        self.invoke_go(page as i32, focus);
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
