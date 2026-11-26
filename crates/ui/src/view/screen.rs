//! The window's page, by name.

use crate::{AppWindow, PageId};

pub(crate) trait Screen {
    fn on_page(&self, page: PageId) -> bool;
    fn go_to(&self, page: PageId, focus: i32);
}

impl Screen for AppWindow {
    fn on_page(&self, page: PageId) -> bool {
        self.get_page() == page
    }
    fn go_to(&self, page: PageId, focus: i32) {
        self.invoke_go(page, focus);
    }
}

/// How `--page` and the `page` step spell a page; the match is exhaustive, so a new page cannot be left out.
pub(crate) fn page_name(page: PageId) -> &'static str {
    match page {
        PageId::Home => "home",
        PageId::Inputs => "inputs",
        PageId::Menu => "menu",
        PageId::Display => "display",
        PageId::Power => "power",
        PageId::Wifi => "wifi",
        PageId::Keyboard => "keyboard",
        PageId::Status => "status",
        PageId::JoinFailed => "join-failed",
        PageId::Tracks => "tracks",
        PageId::FeedbackLab => "feedback-lab",
        PageId::Integrations => "integrations",
    }
}

const PAGES: [PageId; 12] = [
    PageId::Home,
    PageId::Inputs,
    PageId::Menu,
    PageId::Display,
    PageId::Power,
    PageId::Wifi,
    PageId::Keyboard,
    PageId::Status,
    PageId::JoinFailed,
    PageId::Tracks,
    PageId::FeedbackLab,
    PageId::Integrations,
];

pub(crate) fn page_by_name(name: &str) -> Option<PageId> {
    PAGES.into_iter().find(|p| page_name(*p) == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_by_name() {
        assert_eq!(page_by_name("tracks"), Some(PageId::Tracks));
        assert_eq!(page_by_name("feedback-lab"), Some(PageId::FeedbackLab));
        assert_eq!(page_by_name("12"), None);
        for page in PAGES {
            assert_eq!(page_by_name(page_name(page)), Some(page));
        }
    }
}
