//! The model the stills start from: the demo's data, as the session would show it.

use crate::app::model::{InputEntry, Model};
use crate::remote::health::{self, Link};
use crate::remote::wifi::demo_networks;
use r3cui_sdk::api::{Health, Status};
use crate::app::demo::data::{self, AUDIO, FILM_LENGTH, FILM_TITLE, INPUTS, KODI, LEVEL_LIMIT, LINKS, RESUME_AT, START_INPUT, START_LEVEL, WIFI_BAND, WIFI_HOME};
use std::ops::{Deref, DerefMut};

/// The model a still shows, and the one thing the session derives from drivers that a still sets by hand.
pub(super) struct Scene {
    model: Model,
    pub(super) links_down: bool,
}

impl Deref for Scene {
    type Target = Model;
    fn deref(&self) -> &Model {
        &self.model
    }
}

impl DerefMut for Scene {
    fn deref_mut(&mut self) -> &mut Model {
        &mut self.model
    }
}

impl Scene {
    /// `UC_DEMO_TITLE`, `UC_DEMO_SOURCE` and `UC_DEMO_FORMAT` pick what it shows, for previews of other films.
    pub(super) fn new() -> Self {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let model = Model {
            volume: START_LEVEL,
            has_volume: true,
            input: var("UC_DEMO_SOURCE").unwrap_or_else(|| START_INPUT.into()),
            inputs: INPUTS
                .into_iter()
                .map(|s| InputEntry { id: s.into(), name: s.into(), icon: (s == KODI).then_some(kodi::driver::SPEC.icon) })
                .collect(),
            title: var("UC_DEMO_TITLE").unwrap_or_else(|| FILM_TITLE.into()),
            duration: FILM_LENGTH.as_secs_f64(),
            position: RESUME_AT.as_secs_f64(),
            media_format: var("UC_DEMO_FORMAT").unwrap_or_else(|| AUDIO[0].1.into()),
            battery: data::BATTERY,
            volume_max: LEVEL_LIMIT,
            awake: true,
            power_on: true,
            wifi_ssid: WIFI_HOME.into(),
            wifi: demo_networks(WIFI_HOME),
            wifi_band: WIFI_BAND.into(),
            ..Default::default()
        };
        Self { model, links_down: false }
    }

    pub(super) fn play(&mut self) {
        self.model.title = Self::new().model.title;
    }

    /// The model with the status rows and the stage worked out from the rest, as the session does.
    pub(super) fn shown(&self) -> Model {
        let health_now = if self.links_down { Health::Down } else { Health::Up };
        let links: Vec<Link> = LINKS.iter().map(|name| Link { name: (*name).into(), status: Status::new(health_now), in_use: true }).collect();
        let mut m = self.model.clone();
        m.staged = m.input.eq_ignore_ascii_case(KODI);
        (m.status, m.warning) = health::health(&m.wifi_ssid, &links);
        m
    }
}
