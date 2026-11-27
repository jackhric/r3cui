//! The health of each link the remote depends on, for the home screen's warning.

use r3cui_sdk::api::{Health, Status};

/// A driver and how it is doing.
pub struct Link {
    pub name: String,
    pub status: Status,
    /// Global control or the input on now relies on it, so its trouble is worth a warning.
    pub in_use: bool,
}

/// The one-line warning for the home screen, empty when all is well. The first broken link in the chain is the
/// cause; the rest are consequences.
pub fn warning(wifi_ssid: &str, links: &[Link]) -> String {
    if wifi_ssid.is_empty() {
        return "Wi-Fi disconnected".into();
    }
    links
        .iter()
        .find(|l| l.in_use && l.status.health == Health::Down)
        .map(|l| if l.status.detail.is_empty() { format!("{} unreachable", l.name) } else { format!("{}: {}", l.name, l.status.detail) })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(name: &str, health: Health, in_use: bool) -> Link {
        Link { name: name.into(), status: Status::new(health), in_use }
    }

    #[test]
    fn healthy_has_no_warning() {
        assert_eq!(warning("home", &[link("Home Assistant", Health::Up, true), link("Kodi", Health::Up, true)]), "");
    }

    #[test]
    fn no_wifi_is_the_cause() {
        assert_eq!(warning("", &[link("Kodi", Health::Down, true)]), "Wi-Fi disconnected");
    }

    #[test]
    fn trouble_only_matters_where_it_is_used() {
        assert_eq!(warning("home", &[link("Kodi", Health::Down, true)]), "Kodi unreachable");
        assert_eq!(warning("home", &[link("Kodi", Health::Down, false)]), "");
        let rejected = Link { name: "Home Assistant".into(), status: Status::with(Health::Down, "Token rejected"), in_use: true };
        assert_eq!(warning("home", &[rejected]), "Home Assistant: Token rejected");
    }
}
