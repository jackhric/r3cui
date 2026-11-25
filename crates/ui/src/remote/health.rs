//! The health of each link the remote depends on, for the status page and the home screen's warning.

use drivers::api::{Health, Status};

#[derive(Clone, Debug, PartialEq)]
pub struct StatusLine {
    pub label: String,
    pub value: String,
    /// 0 working, 1 broken, 2 unknown or not set up
    pub level: i32,
}

/// A driver as the status page lists it.
pub struct Link {
    pub name: String,
    pub status: Status,
    /// Global control or the input on now relies on it, so its trouble is worth a warning.
    pub in_use: bool,
}

/// The status page's rows, and the one-line warning for the home screen (empty when all is well).
pub fn health(wifi_ssid: &str, links: &[Link]) -> (Vec<StatusLine>, String) {
    let wifi_up = !wifi_ssid.is_empty();
    let mut rows = vec![StatusLine {
        label: "Wi-Fi".into(),
        value: if wifi_up { wifi_ssid.into() } else { "Disconnected".into() },
        level: if wifi_up { 0 } else { 1 },
    }];
    for l in links {
        rows.push(if !wifi_up {
            StatusLine { label: l.name.clone(), value: "Unknown".into(), level: 2 }
        } else {
            let level = match l.status.health {
                Health::Up => 0,
                Health::Down => 1,
                Health::Connecting | Health::NotSetUp => 2,
            };
            StatusLine { label: l.name.clone(), value: l.status.word().into(), level }
        });
    }
    // The first broken link in the chain is the cause; the rest are consequences.
    let warning = if !wifi_up {
        "Wi-Fi disconnected".to_string()
    } else {
        links
            .iter()
            .find(|l| l.in_use && l.status.health == Health::Down)
            .map(|l| if l.status.detail.is_empty() { format!("{} unreachable", l.name) } else { format!("{}: {}", l.name, l.status.detail) })
            .unwrap_or_default()
    };
    (rows, warning)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(name: &str, health: Health, in_use: bool) -> Link {
        Link { name: name.into(), status: Status::new(health), in_use }
    }

    #[test]
    fn healthy_has_no_warning() {
        let (rows, warning) = health("home", &[link("Home Assistant", Health::Up, true), link("Kodi", Health::Up, true)]);
        assert!(rows.iter().all(|r| r.level == 0));
        assert_eq!((rows.len(), warning.as_str()), (3, ""));
    }

    #[test]
    fn no_wifi_greys_every_driver() {
        let (rows, warning) = health("", &[link("Kodi", Health::Down, true)]);
        assert_eq!((rows[0].level, rows[1].level, rows[1].value.as_str()), (1, 2, "Unknown"));
        assert_eq!(warning, "Wi-Fi disconnected");
    }

    #[test]
    fn trouble_only_matters_where_it_is_used() {
        assert_eq!(health("home", &[link("Kodi", Health::Down, true)]).1, "Kodi unreachable");
        assert_eq!(health("home", &[link("Kodi", Health::Down, false)]).1, "");
        let rejected = Link { name: "Home Assistant".into(), status: Status::with(Health::Down, "Token rejected"), in_use: true };
        assert_eq!(health("home", &[rejected]).1, "Home Assistant: Token rejected");
    }
}
