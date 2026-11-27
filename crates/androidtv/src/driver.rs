//! The Android TV driver, on r3cui's own client (`crates/androidtv`): the box's keys in an input's remote slot, the
//! app in front on its screen slot, and its power key as global power.

use r3cui_sdk::api::*;
use r3cui_sdk::DriverEntry;
use r3cui_sdk::run::follow;
use crate::{connect, AndroidTvHandle, Credentials, Endpoint, KeyCode, Link, TvState};
use std::future::Future;
use std::pin::Pin;
use tokio::sync::{mpsc, watch};

pub mod field {
    pub const HOST: &str = "host";
    pub const CERT_PEM: &str = "cert_pem";
    pub const KEY_PEM: &str = "key_pem";
}

/// What the box lists the paired remote as.
const CLIENT_NAME: &str = "r3cui";
/// The box shows six hexadecimal symbols.
const CODE_LEN: usize = 6;

pub static SPEC: Spec = Spec {
    id: "androidtv",
    name: "Android TV",
    // A television outline on a stand.
    icon: "M18.4395 5.5586c-.675 1.1664-1.352 2.3318-2.0274 3.498-.0366-.0155-.0742-.0286-.1113-.043-1.8249-.6957-3.484-.8-4.42-.787-1.8551.0185-3.3544.4643-4.2597.8203-.084-.1494-1.7526-3.021-2.0215-3.4864a1.1451 1.1451 0 0 0-.1406-.1914c-.3312-.364-.9054-.4859-1.379-.203-.475.282-.7136.9361-.3886 1.5019 1.9466 3.3696-.0966-.2158 1.9473 3.3593.0172.031-.4946.2642-1.3926 1.0177C2.8987 12.176.452 14.772 0 18.9902h24c-.119-1.1108-.3686-2.099-.7461-3.0683-.7438-1.9118-1.8435-3.2928-2.7402-4.1836a12.1048 12.1048 0 0 0-2.1309-1.6875c.6594-1.122 1.312-2.2559 1.9649-3.3848.2077-.3615.1886-.7956-.0079-1.1191a1.1001 1.1001 0 0 0-.8515-.5332c-.5225-.0536-.9392.3128-1.0488.5449zm-.0391 8.461c.3944.5926.324 1.3306-.1563 1.6503-.4799.3197-1.188.0985-1.582-.4941-.3944-.5927-.324-1.3307.1563-1.6504.4727-.315 1.1812-.1086 1.582.4941zM7.207 13.5273c.4803.3197.5506 1.0577.1563 1.6504-.394.5926-1.1038.8138-1.584.4941-.48-.3197-.5503-1.0577-.1563-1.6504.4008-.6021 1.1087-.8106 1.584-.4941z",
    fields: &[
        Field { key: field::HOST, label: "Address", kind: FieldKind::Text, required: true, default: None },
        Field { key: field::CERT_PEM, label: "Certificate", kind: FieldKind::Credential, required: false, default: None },
        Field { key: field::KEY_PEM, label: "Key", kind: FieldKind::Credential, required: false, default: None },
    ],
    provides: Provides { global: false, remote: true, ui: true, power: true, voice: false },
    address: Some(field::HOST),
    pairing: Some(CodePairing { code_len: CODE_LEN }),
};

/// What the app lists to carry this integration.
pub const INTEGRATION: r3cui_sdk::Integration = r3cui_sdk::Integration { spec: &SPEC, start: |e| std::sync::Arc::new(AndroidTv::start(e)) };

pub struct AndroidTv {
    status: watch::Receiver<Status>,
    remote: mpsc::UnboundedSender<RemoteKey>,
    ui: UiPort,
    power: PowerPort,
    endpoint: Option<Endpoint>,
    credentials: Option<Credentials>,
}

impl AndroidTv {
    pub fn start(entry: &DriverEntry) -> Self {
        let credentials = entry.text(field::CERT_PEM).zip(entry.text(field::KEY_PEM)).map(|(cert_pem, key_pem)| Credentials { cert_pem, key_pem });
        Self::at(entry.text(field::HOST).map(Endpoint::new), credentials)
    }

    /// Starts on a given endpoint, which may have ports of its own.
    pub fn at(endpoint: Option<Endpoint>, credentials: Option<Credentials>) -> Self {
        let (remote, mut keys) = mpsc::unbounded_channel();
        let (stage, _ui_intents, ui) = port(Stage::default());
        let (power_view, mut power_intents, power) = port(None);
        let this = |status| Self { status, remote: remote.clone(), ui: ui.clone(), power: power.clone(), endpoint: endpoint.clone(), credentials: credentials.clone() };
        let Some(at) = endpoint.clone() else { return this(watch::channel(Status::with(Health::NotSetUp, "Address missing")).1) };
        let Some(creds) = credentials.clone() else { return this(watch::channel(Status::new(Health::NotPaired)).1) };
        let tv = match connect(at.clone(), &creds) {
            Ok(tv) => tv,
            Err(e) => {
                tracing::warn!("androidtv {}: {e}", at.host);
                return this(watch::channel(Status::with(Health::NotPaired, "Credentials unusable")).1);
            }
        };
        let (status_tx, status) = watch::channel(Status::new(Health::Connecting));
        follow(tv.watch(), status_tx, |s: &TvState| status_of(s.link));
        follow(tv.watch(), stage, stage_of);
        follow(tv.watch(), power_view, |s: &TvState| (s.link == Link::Up).then_some(Power { on: s.on }));
        tokio::spawn({
            let tv = tv.clone();
            async move {
                while let Some(key) = keys.recv().await {
                    press(&tv, key_code(key));
                }
            }
        });
        tokio::spawn(async move {
            while let Some(PowerIntent::Set(on)) = power_intents.recv().await {
                if tv.state().on != on {
                    press(&tv, KeyCode::Power);
                }
            }
        });
        this(status)
    }
}

impl Driver for AndroidTv {
    fn spec(&self) -> &'static Spec {
        &SPEC
    }

    fn status(&self) -> watch::Receiver<Status> {
        self.status.clone()
    }

    fn remote(&self) -> Option<mpsc::UnboundedSender<RemoteKey>> {
        Some(self.remote.clone())
    }

    fn ui(&self) -> Option<UiPort> {
        Some(self.ui.clone())
    }

    fn power(&self, _target: &str) -> Option<PowerPort> {
        Some(self.power.clone())
    }

    fn pair(&self) -> Option<PairStart> {
        let endpoint = self.endpoint.clone()?;
        let existing = self.credentials.clone();
        Some(Box::pin(async move {
            let credentials = match existing {
                Some(c) => c,
                None => tokio::task::spawn_blocking(Credentials::generate).await.map_err(|e| failed(&e))?.map_err(|e| failed(&e))?,
            };
            let pairing = crate::pair(endpoint, &credentials, CLIENT_NAME).await.map_err(|e| failed(&e))?;
            Ok(Box::new(Pairing { pairing, credentials }) as Box<dyn Pairer>)
        }))
    }
}

struct Pairing {
    pairing: crate::Pairing,
    credentials: Credentials,
}

impl Pairer for Pairing {
    fn finish<'a>(&'a mut self, code: &'a str) -> Pin<Box<dyn Future<Output = Result<Paired, PairError>> + Send + 'a>> {
        Box::pin(async move {
            match self.pairing.finish(code).await {
                Ok(()) => Ok(vec![(field::CERT_PEM, self.credentials.cert_pem.clone()), (field::KEY_PEM, self.credentials.key_pem.clone())]),
                Err(crate::Error::WrongCode | crate::Error::InvalidCode) => Err(PairError::WrongCode),
                Err(e) => Err(failed(&e)),
            }
        })
    }
}

fn failed(e: &dyn std::fmt::Display) -> PairError {
    PairError::Failed(e.to_string())
}

fn press(tv: &AndroidTvHandle, key: KeyCode) {
    if let Err(e) = tv.press(key) {
        tracing::warn!("androidtv {key:?}: {e}");
    }
}

fn status_of(link: Link) -> Status {
    Status::new(match link {
        Link::Connecting => Health::Connecting,
        Link::Up => Health::Up,
        Link::NotPaired => Health::NotPaired,
        Link::Down => Health::Down,
    })
}

fn stage_of(s: &TvState) -> Stage {
    let app = if s.on { s.current_app.as_deref().map(|package| App { name: app_name(package) }) } else { None };
    Stage { app, ..Stage::default() }
}

fn key_code(key: RemoteKey) -> KeyCode {
    match key {
        RemoteKey::Up => KeyCode::DpadUp,
        RemoteKey::Down => KeyCode::DpadDown,
        RemoteKey::Left => KeyCode::DpadLeft,
        RemoteKey::Right => KeyCode::DpadRight,
        RemoteKey::Select => KeyCode::DpadCenter,
        RemoteKey::Back => KeyCode::Back,
        RemoteKey::Menu => KeyCode::Menu,
        RemoteKey::ChannelUp => KeyCode::ChannelUp,
        RemoteKey::ChannelDown => KeyCode::ChannelDown,
        RemoteKey::Previous => KeyCode::MediaPrevious,
        RemoteKey::PlayPause => KeyCode::MediaPlayPause,
        RemoteKey::Next => KeyCode::MediaNext,
        RemoteKey::Stop => KeyCode::MediaStop,
        RemoteKey::Record => KeyCode::MediaRecord,
    }
}

const HOME: &str = "Home";

const APPS: &[(&str, &str)] = &[
    ("com.google.android.youtube.tv", "YouTube"),
    ("com.google.android.youtube.tvunplugged", "YouTube TV"),
    ("com.netflix.ninja", "Netflix"),
    ("com.plexapp.android", "Plex"),
    ("org.xbmc.kodi", "Kodi"),
    ("com.amazon.amazonvideo.livingroom", "Prime Video"),
    ("com.disney.disneyplus", "Disney+"),
    ("com.spotify.tv.android", "Spotify"),
    ("tv.twitch.android.app", "Twitch"),
    ("org.jellyfin.androidtv", "Jellyfin"),
    ("com.google.android.tvlauncher", HOME),
    ("com.google.android.apps.tv.launcherx", HOME),
    ("com.google.android.leanbacklauncher", HOME),
];

/// A program as a person names it: the known ones by their name, any other by the last part of its package.
fn app_name(package: &str) -> String {
    if let Some((_, name)) = APPS.iter().find(|(p, _)| *p == package) {
        return (*name).to_string();
    }
    let last = package.rsplit('.').next().unwrap_or(package);
    let mut chars = last.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use r3cui_sdk::testing::until;
    use crate::testing::FakeTv;
    use crate::Direction;
    use std::time::Duration;

    async fn paired() -> (FakeTv, Credentials) {
        let tv = FakeTv::start().await;
        let credentials = tokio::task::spawn_blocking(Credentials::generate).await.unwrap().unwrap();
        let mut pairing = crate::pair(tv.endpoint(), &credentials, CLIENT_NAME).await.unwrap();
        let shown = tv.code().wait_for(Option::is_some).await.unwrap().clone().unwrap();
        pairing.finish(&shown).await.unwrap();
        (tv, credentials)
    }

    async fn driver(tv: &FakeTv, credentials: &Credentials) -> AndroidTv {
        let d = AndroidTv::at(Some(tv.endpoint()), Some(credentials.clone()));
        until(&mut d.status(), |s| s.health == Health::Up).await;
        d
    }

    async fn keys_reach(tv: &FakeTv, wanted: &[(KeyCode, Direction)]) {
        for _ in 0..100 {
            if tv.keys() == wanted {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(tv.keys(), wanted);
    }

    #[test]
    fn apps_are_named_as_people_know_them() {
        assert_eq!(app_name("com.netflix.ninja"), "Netflix");
        assert_eq!(app_name("com.google.android.tvlauncher"), "Home");
        assert_eq!(app_name("com.example.weatherbox"), "Weatherbox");
        assert_eq!(app_name("plain"), "Plain");
    }

    #[tokio::test]
    async fn no_address_is_not_set_up_and_no_credentials_is_not_paired() {
        let d = AndroidTv::at(None, None);
        assert_eq!(d.status().borrow().health, Health::NotSetUp);
        assert!(d.pair().is_none(), "nothing to pair with");
        let d = AndroidTv::at(Some(Endpoint::new("127.0.0.1")), None);
        assert_eq!(d.status().borrow().word(), "Needs pairing");
        assert!(d.pair().is_some());
    }

    #[tokio::test]
    async fn keys_arrive_mapped() {
        let (tv, credentials) = paired().await;
        let d = driver(&tv, &credentials).await;
        for key in [RemoteKey::Up, RemoteKey::Select, RemoteKey::PlayPause] {
            d.remote().unwrap().send(key).unwrap();
        }
        keys_reach(&tv, &[(KeyCode::DpadUp, Direction::Short), (KeyCode::DpadCenter, Direction::Short), (KeyCode::MediaPlayPause, Direction::Short)]).await;
    }

    #[tokio::test]
    async fn power_follows_the_box_and_is_pressed_only_on_a_difference() {
        let (tv, credentials) = paired().await;
        tv.set_on(true);
        let d = driver(&tv, &credentials).await;
        let power = d.power("").unwrap();
        let mut view = power.view.clone();
        until(&mut view, |p| *p == Some(Power { on: true })).await;
        power.send(PowerIntent::Set(true));
        power.send(PowerIntent::Set(false));
        keys_reach(&tv, &[(KeyCode::Power, Direction::Short)]).await;
        tv.set_on(false);
        until(&mut view, |p| *p == Some(Power { on: false })).await;
    }

    #[tokio::test]
    async fn the_screen_shows_the_app_in_front_while_on() {
        let (tv, credentials) = paired().await;
        let d = driver(&tv, &credentials).await;
        let mut stage = d.ui().unwrap().view.clone();
        tv.set_on(true);
        tv.set_app("com.netflix.ninja");
        until(&mut stage, |s| s.app == Some(App { name: "Netflix".into() })).await;
        tv.set_on(false);
        until(&mut stage, |s| *s == Stage::default()).await;
    }

    #[tokio::test]
    async fn a_box_that_forgot_us_needs_pairing() {
        let (tv, credentials) = paired().await;
        let d = driver(&tv, &credentials).await;
        tv.forget_clients();
        tv.kick();
        until(&mut d.status(), |s| s.health == Health::NotPaired).await;
    }

    #[tokio::test]
    async fn pairing_hands_back_the_credentials_and_a_wrong_code_can_be_retyped() {
        let tv = FakeTv::start().await;
        let d = AndroidTv::at(Some(tv.endpoint()), None);
        let Ok(mut pairer) = d.pair().unwrap().await else { panic!("pairing starts") };
        let shown = tv.code().wait_for(Option::is_some).await.unwrap().clone().unwrap();
        let wrong = if shown.starts_with("00") { "010000" } else { "000000" };
        assert_eq!(pairer.finish(wrong).await, Err(PairError::WrongCode));
        let stored = pairer.finish(&shown).await.unwrap();
        assert_eq!(stored.iter().map(|(k, _)| *k).collect::<Vec<_>>(), [field::CERT_PEM, field::KEY_PEM]);
        assert_eq!(tv.paired_clients(), 1);
    }
}
