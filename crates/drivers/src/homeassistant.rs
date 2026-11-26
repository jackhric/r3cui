//! The Home Assistant driver, on r3cui's own client: a media player as global volume and source, and the helper
//! entities that steer the key backlight and cap the volume.

use crate::api::*;
use crate::config::DriverEntry;
use crate::run::{follow, in_order};
use homeassistant::{EntityId, EntityState, HaConfig, HaHandle, HaService, HaState, Light, Link, MediaPlayer};
use std::collections::BTreeSet;
use std::sync::Mutex;
use tokio::sync::watch;

pub static SPEC: Spec = Spec {
    id: "homeassistant",
    name: "Home Assistant",
    // Simple Icons (CC0); the mark is Home Assistant's.
    icon: "M22.939 10.627 13.061.749a1.505 1.505 0 0 0-2.121 0l-9.879 9.878C.478 11.21 0 12.363 0 13.187v9c0 .826.675 1.5 1.5 1.5h9.227l-4.063-4.062a2.034 2.034 0 0 1-.664.113c-1.13 0-2.05-.92-2.05-2.05s.92-2.05 2.05-2.05 2.05.92 2.05 2.05c0 .233-.041.456-.113.665l3.163 3.163V9.928a2.05 2.05 0 0 1-1.15-1.84c0-1.13.92-2.05 2.05-2.05s2.05.92 2.05 2.05a2.05 2.05 0 0 1-1.15 1.84v8.127l3.146-3.146A2.051 2.051 0 0 1 18 12.239c1.13 0 2.05.92 2.05 2.05s-.92 2.05-2.05 2.05c-.25 0-.488-.047-.709-.13L12.9 20.602v3.088h9.6c.825 0 1.5-.675 1.5-1.5v-9c0-.825-.477-1.977-1.061-2.561z",
    fields: &[
        Field { key: "url", label: "Address", kind: FieldKind::Url, required: true, default: Some("http://homeassistant.local:8123") },
        Field { key: "token", label: "Access token", kind: FieldKind::Secret, required: true, default: None },
        Field { key: "volume_limit", label: "Volume limit", kind: FieldKind::Target(TargetKind::Sensor), required: false, default: None },
        Field { key: "keys_light", label: "Key light", kind: FieldKind::Target(TargetKind::Light), required: false, default: None },
        Field { key: "keys_mode", label: "Key light mode", kind: FieldKind::Target(TargetKind::Select), required: false, default: None },
    ],
    provides: Provides { global: true, remote: false, ui: false },
};

/// Full scale when there is no volume-limit sensor.
const FULL: i32 = 100;

pub struct HomeAssistant {
    ha: HaHandle,
    status: watch::Receiver<Status>,
    key_light: Option<watch::Receiver<Option<KeyLight>>>,
    limit: Option<EntityId>,
    /// Every entity some port reads, so one subscription covers them all.
    watched: Mutex<BTreeSet<EntityId>>,
}

impl HomeAssistant {
    pub fn start(entry: &DriverEntry) -> Self {
        let id = |key| entry.text(key).map(EntityId::new);
        let (limit, light, mode) = (id("volume_limit"), id("keys_light"), id("keys_mode"));
        let watched: BTreeSet<EntityId> = [&limit, &light, &mode].into_iter().flatten().cloned().collect();
        let cfg = HaConfig::new(entry.text("url").unwrap_or_default(), entry.text("token").unwrap_or_default());
        let ha = HaService::start(cfg, watched.iter().cloned().collect());

        let (status_tx, status) = watch::channel(Status::new(Health::Connecting));
        follow(ha.state(), status_tx, |s: &HaState| status_of(&s.link));
        let key_light = (light.is_some() || mode.is_some()).then(|| {
            let (tx, rx) = watch::channel(None);
            follow(ha.state(), tx, move |s: &HaState| key_light(s, light.as_ref(), mode.as_ref()));
            rx
        });
        Self { ha, status, key_light, limit, watched: Mutex::new(watched) }
    }

    fn watch(&self, id: &EntityId) {
        let mut watched = self.watched.lock().unwrap();
        if watched.insert(id.clone()) {
            self.ha.watch(watched.iter().cloned().collect());
        }
    }
}

fn status_of(link: &Link) -> Status {
    match link {
        Link::Unconfigured => Status::with(Health::NotSetUp, "Address or token missing"),
        Link::Connecting => Status::new(Health::Connecting),
        Link::Up => Status::new(Health::Up),
        Link::Down { .. } => Status::new(Health::Down),
        Link::Rejected(_) => Status::with(Health::Down, "Token rejected"),
    }
}

fn volume_of(s: &HaState, target: &EntityId, limit: Option<&EntityId>) -> Option<Volume> {
    let player = MediaPlayer(s.entities.get(target)?);
    let max = limit.and_then(|l| s.entities.get(l)?.number()).map_or(FULL, |v| (v.round() as i32).clamp(1, FULL));
    Some(Volume { level: (player.volume_level()? * 100.0).round() as i32, max, muted: player.muted().unwrap_or(false) })
}

fn sources_of(s: &HaState, target: &EntityId) -> Option<Sources> {
    let player = MediaPlayer(s.entities.get(target)?);
    Some(Sources {
        on: player.is_on(),
        current: player.source().unwrap_or_default().to_string(),
        list: player.source_list().into_iter().map(String::from).collect(),
    })
}

/// Without the mode helper the light alone decides: on means its colour, off means follow the remote.
fn key_light(s: &HaState, light: Option<&EntityId>, mode: Option<&EntityId>) -> Option<KeyLight> {
    let colour = light.and_then(|l| s.entities.get(l)).map(Light).filter(Light::is_on).map(|l| {
        let (h, sat) = l.hs_color().unwrap_or((0.0, 0.0));
        hsv(h, sat / 100.0, f64::from(l.brightness().unwrap_or(255)) / 255.0)
    });
    let option = mode.and_then(|m| s.entities.get(m)).map(|e| e.state.to_lowercase()).unwrap_or_default();
    let mode = match option.as_str() {
        "off" => KeyMode::Off,
        "custom" => KeyMode::Custom,
        "" if colour.is_some() => KeyMode::Custom,
        _ => KeyMode::Follow,
    };
    Some(KeyLight { mode, colour })
}

/// Hue in degrees; saturation and value 0 to 1.
fn hsv(h: f64, s: f64, v: f64) -> (u8, u8, u8) {
    let f = |n: f64| {
        let k = (n + h / 60.0) % 6.0;
        ((v - v * s * k.min(4.0 - k).clamp(0.0, 1.0)) * 255.0).round() as u8
    };
    (f(5.0), f(3.0), f(1.0))
}

impl Driver for HomeAssistant {
    fn spec(&self) -> &'static Spec {
        &SPEC
    }

    fn status(&self) -> watch::Receiver<Status> {
        self.status.clone()
    }

    fn volume(&self, target: &str) -> Option<VolumePort> {
        let target = EntityId::from(target);
        self.watch(&target);
        let limit = self.limit.clone();
        // Filled from what is known now, so a new binding shows at once.
        let (view, mut intents, port) = port(volume_of(&self.ha.state().borrow(), &target, limit.as_ref()));
        follow(self.ha.state(), view, {
            let target = target.clone();
            move |s: &HaState| volume_of(s, &target, limit.as_ref())
        });
        let ha = self.ha.clone();
        tokio::spawn(async move {
            while let Some(intent) = intents.recv().await {
                let (ha, t) = (ha.clone(), target.clone());
                let what = format!("{t} {intent:?}");
                match intent {
                    VolumeIntent::Up => in_order(what, async move { ha.volume_up(&t).await }).await,
                    VolumeIntent::Down => in_order(what, async move { ha.volume_down(&t).await }).await,
                    VolumeIntent::Set(level) => in_order(what, async move { ha.volume_set(&t, f64::from(level) / 100.0).await }).await,
                    VolumeIntent::Mute(muted) => in_order(what, async move { ha.volume_mute(&t, muted).await }).await,
                }
            }
        });
        Some(port)
    }

    fn source(&self, target: &str) -> Option<SourcePort> {
        let target = EntityId::from(target);
        self.watch(&target);
        let (view, mut intents, port) = port(sources_of(&self.ha.state().borrow(), &target));
        follow(self.ha.state(), view, {
            let target = target.clone();
            move |s: &HaState| sources_of(s, &target)
        });
        let ha = self.ha.clone();
        tokio::spawn(async move {
            while let Some(intent) = intents.recv().await {
                let (ha, t) = (ha.clone(), target.clone());
                let what = format!("{t} {intent:?}");
                match intent {
                    SourceIntent::Select(name) => in_order(what, async move { ha.select_source(&t, &name).await }).await,
                    SourceIntent::Power(true) => in_order(what, async move { ha.turn_on(&t).await }).await,
                    SourceIntent::Power(false) => in_order(what, async move { ha.turn_off(&t).await }).await,
                }
            }
        });
        Some(port)
    }

    fn key_light(&self) -> Option<watch::Receiver<Option<KeyLight>>> {
        self.key_light.clone()
    }

    fn targets(&self, kind: TargetKind) -> Targets<'_> {
        Box::pin(async move {
            let domains: &[&str] = match kind {
                TargetKind::MediaPlayer => &["media_player"],
                TargetKind::Light => &["light"],
                TargetKind::Select => &["select", "input_select"],
                TargetKind::Sensor => &["sensor", "input_number"],
            };
            let states = self.ha.states().await.map_err(|e| tracing::warn!("home assistant entities: {e}")).unwrap_or_default();
            let mut found: Vec<Target> = states
                .into_iter()
                .filter(|(id, _)| domains.contains(&id.domain()))
                .map(|(id, s): (EntityId, EntityState)| Target { name: s.friendly_name().unwrap_or(id.as_str()).to_string(), id: id.as_str().to_string() })
                .collect();
            found.sort_by_key(|t| t.name.to_lowercase());
            found
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Modules;
    use homeassistant::testing::FakeHa;
    use serde_json::json;
    use std::time::Duration;

    async fn until<T: Clone>(rx: &mut watch::Receiver<T>, ok: impl Fn(&T) -> bool) -> T {
        tokio::time::timeout(Duration::from_secs(5), rx.wait_for(|v| ok(v))).await.expect("in time").expect("open").clone()
    }

    fn receiver() -> serde_json::Value {
        json!({ "source_list": ["KODI", "NET", "PLAYSTATION 5"], "volume_level": 0.56, "is_volume_muted": false, "source": "KODI", "supported_features": 69004 })
    }

    async fn driver(fake: &FakeHa) -> HomeAssistant {
        let cfg = fake.config();
        let mut m = Modules::default();
        m.add_driver("homeassistant", Default::default());
        m.set_field("homeassistant", "url", &cfg.url);
        m.set_field("homeassistant", "token", &cfg.token);
        m.set_field("homeassistant", "volume_limit", "sensor.limit");
        m.set_field("homeassistant", "keys_light", "light.keys");
        HomeAssistant::start(&m.drivers["homeassistant"])
    }

    #[tokio::test]
    async fn the_receiver_as_volume_and_source() {
        let fake = FakeHa::start("token").await;
        fake.set_entity("media_player.receiver", "on", receiver());
        fake.set_entity("sensor.limit", "72", json!({}));
        let ha = driver(&fake).await;
        let mut status = ha.status();
        until(&mut status, |s| s.health == Health::Up).await;

        let volume = ha.volume("media_player.receiver").unwrap();
        let v = until(&mut volume.view.clone(), Option::is_some).await.unwrap();
        assert_eq!((v.level, v.max, v.muted), (56, 72, false));
        let source = ha.source("media_player.receiver").unwrap();
        let s = until(&mut source.view.clone(), Option::is_some).await.unwrap();
        assert_eq!((s.on, s.current.as_str(), s.list.len()), (true, "KODI", 3));

        volume.send(VolumeIntent::Set(40));
        until(&mut volume.view.clone(), |v| v.as_ref().is_some_and(|v| v.level == 40)).await;
        volume.send(VolumeIntent::Mute(true));
        until(&mut volume.view.clone(), |v| v.as_ref().is_some_and(|v| v.muted)).await;
        source.send(SourceIntent::Select("NET".into()));
        until(&mut source.view.clone(), |s| s.as_ref().is_some_and(|s| s.current == "NET")).await;
        source.send(SourceIntent::Power(false));
        until(&mut source.view.clone(), |s| s.as_ref().is_some_and(|s| !s.on)).await;
        let services: Vec<String> = fake.calls().into_iter().map(|(d, s, _)| format!("{d}.{s}")).collect();
        assert_eq!(services, ["media_player.volume_set", "media_player.volume_mute", "media_player.select_source", "media_player.turn_off"]);
    }

    #[tokio::test]
    async fn a_bad_token_says_so() {
        let fake = FakeHa::start("right").await;
        let mut m = Modules::default();
        m.add_driver("homeassistant", Default::default());
        m.set_field("homeassistant", "url", &fake.config().url);
        m.set_field("homeassistant", "token", "wrong");
        let ha = HomeAssistant::start(&m.drivers["homeassistant"]);
        let s = until(&mut ha.status(), |s| s.health == Health::Down).await;
        assert_eq!(s.word(), "Token rejected");
    }

    #[tokio::test]
    async fn the_key_light_follows_its_helpers() {
        let fake = FakeHa::start("token").await;
        fake.set_entity("light.keys", "on", json!({ "hs_color": [240.0, 100.0], "brightness": 255 }));
        let ha = driver(&fake).await;
        let mut keys = ha.key_light().unwrap();
        let k = until(&mut keys, |k| k.is_some_and(|k| k.colour.is_some())).await.unwrap();
        assert_eq!((k.mode, k.colour), (KeyMode::Custom, Some((0, 0, 255))));
        fake.set_entity("light.keys", "off", json!({}));
        let k = until(&mut keys, |k| k.is_some_and(|k| k.colour.is_none())).await.unwrap();
        assert_eq!(k.mode, KeyMode::Follow);
    }

    #[tokio::test]
    async fn pickers_list_entities_by_kind() {
        let fake = FakeHa::start("token").await;
        fake.set_entity("media_player.receiver", "on", json!({ "friendly_name": "Theater Receiver" }));
        fake.set_entity("light.keys", "on", json!({}));
        let ha = driver(&fake).await;
        until(&mut ha.status(), |s| s.health == Health::Up).await;
        let players = ha.targets(TargetKind::MediaPlayer).await;
        assert_eq!(players, [Target { id: "media_player.receiver".into(), name: "Theater Receiver".into() }]);
    }

    #[test]
    fn hsv_to_rgb() {
        assert_eq!(hsv(0.0, 1.0, 1.0), (255, 0, 0));
        assert_eq!(hsv(120.0, 1.0, 1.0), (0, 255, 0));
        assert_eq!(hsv(240.0, 1.0, 0.5), (0, 0, 128));
        assert_eq!(hsv(0.0, 0.0, 1.0), (255, 255, 255));
    }
}
