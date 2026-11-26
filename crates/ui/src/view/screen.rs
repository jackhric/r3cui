//! The window's page, by name.

use slint::ComponentHandle;
use crate::Nav;
use crate::{AppWindow, PageId};

pub(crate) trait Screen {
    fn on_page(&self, page: PageId) -> bool;
    fn go_to(&self, page: PageId, focus: i32);
}

impl Screen for AppWindow {
    fn on_page(&self, page: PageId) -> bool {
        self.global::<Nav>().get_page() == page
    }
    fn go_to(&self, page: PageId, focus: i32) {
        self.global::<Nav>().invoke_go(page, focus);
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
        PageId::System => "system",
        PageId::R3Update => "r3-update",
        PageId::R3Installing => "r3-installing",
        PageId::Uninstall => "uninstall",
        PageId::FwUpdate => "fw-update",
        PageId::FwInstalling => "fw-installing",
        PageId::WebConfig => "web-config",
    }
}

const PAGES: [PageId; 19] = [
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
    PageId::System,
    PageId::R3Update,
    PageId::R3Installing,
    PageId::Uninstall,
    PageId::FwUpdate,
    PageId::FwInstalling,
    PageId::WebConfig,
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
