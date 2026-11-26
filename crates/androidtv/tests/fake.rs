use androidtv::testing::{FakeOptions, FakeTv, FAKE_MODEL, FAKE_VENDOR};
use androidtv::{connect, pair, AndroidTvHandle, Credentials, Direction, Error, KeyCode, Link, TvState, Volume};
use std::sync::OnceLock;
use std::time::Duration;

const CLIENT_NAME: &str = "test remote";
const WAIT: Duration = Duration::from_secs(10);

fn credentials() -> Credentials {
    static CREDENTIALS: OnceLock<Credentials> = OnceLock::new();
    CREDENTIALS.get_or_init(|| Credentials::generate().unwrap()).clone()
}

async fn shown_code(tv: &FakeTv) -> String {
    let mut code = tv.code();
    let shown = tokio::time::timeout(WAIT, code.wait_for(Option::is_some)).await.unwrap().unwrap().clone();
    shown.unwrap()
}

async fn paired(tv: &FakeTv) -> Credentials {
    let credentials = credentials();
    let mut pairing = pair(tv.endpoint(), &credentials, CLIENT_NAME).await.unwrap();
    pairing.finish(&shown_code(tv).await).await.unwrap();
    credentials
}

async fn until(handle: &AndroidTvHandle, wanted: impl Fn(&TvState) -> bool) -> TvState {
    let mut state = handle.watch();
    let reached = tokio::time::timeout(WAIT, state.wait_for(|s| wanted(s))).await.expect("the state never got there").unwrap().clone();
    reached
}

async fn eventually(what: &str, mut done: impl FnMut() -> bool) {
    tokio::time::timeout(WAIT, async {
        while !done() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("never happened: {what}"));
}

async fn ready(tv: &FakeTv) -> AndroidTvHandle {
    let handle = connect(tv.endpoint(), &paired(tv).await).unwrap();
    until(&handle, |s| s.link == Link::Up).await;
    handle
}

#[tokio::test]
async fn pairing_with_the_right_code_registers_the_client() {
    let tv = FakeTv::start().await;
    assert_eq!(tv.paired_clients(), 0);
    paired(&tv).await;
    assert_eq!(tv.paired_clients(), 1);
}

#[tokio::test]
async fn a_wrong_code_is_refused_and_pairs_nothing() {
    let tv = FakeTv::start().await;
    let mut pairing = pair(tv.endpoint(), &credentials(), CLIENT_NAME).await.unwrap();
    let code = shown_code(&tv).await;
    let wrong = format!("{}{:02X}", &code[..4], u8::from_str_radix(&code[4..], 16).unwrap() ^ 0xff);
    assert!(matches!(pairing.finish(&wrong).await, Err(Error::WrongCode)));
    assert!(matches!(pairing.finish("zzzzzz").await, Err(Error::InvalidCode)));
    assert_eq!(tv.paired_clients(), 0);
    pairing.finish(&code).await.unwrap();
    assert_eq!(tv.paired_clients(), 1);
}

#[tokio::test]
async fn keys_and_launches_reach_the_tv() {
    let tv = FakeTv::start().await;
    let handle = ready(&tv).await;
    handle.send_key(KeyCode::DpadCenter, Direction::StartLong).unwrap();
    handle.send_key(KeyCode::DpadCenter, Direction::EndLong).unwrap();
    handle.press(KeyCode::MediaPlayPause).unwrap();
    handle.launch("https://www.youtube.com").unwrap();
    eventually("the keys", || tv.keys().len() == 3 && tv.launches().len() == 1).await;
    assert_eq!(tv.keys(), [(KeyCode::DpadCenter, Direction::StartLong), (KeyCode::DpadCenter, Direction::EndLong), (KeyCode::MediaPlayPause, Direction::Short)]);
    assert_eq!(tv.launches(), ["https://www.youtube.com"]);
}

#[tokio::test]
async fn the_tv_state_arrives_and_follows_changes() {
    let tv = FakeTv::start().await;
    let handle = ready(&tv).await;
    let state = until(&handle, |s| s.current_app.is_some() && s.volume.is_some()).await;
    assert!(state.on);
    assert_eq!(state.model.as_deref(), Some(format!("{FAKE_VENDOR} {FAKE_MODEL}").as_str()));
    assert_eq!(state.current_app.as_deref(), Some("com.fake.launcher"));

    tv.set_on(false);
    tv.set_app("com.fake.player");
    let volume = Volume { level: 42, max: 100, muted: true };
    tv.set_volume(volume);
    let state = until(&handle, |s| !s.on && s.current_app.as_deref() == Some("com.fake.player") && s.volume == Some(volume)).await;
    assert_eq!(state.link, Link::Up);
}

#[tokio::test]
async fn pings_are_answered() {
    let tv = FakeTv::start_with(FakeOptions { ping_every: Duration::from_millis(30), ..FakeOptions::default() }).await;
    let _handle = ready(&tv).await;
    eventually("three pongs", || tv.pings_answered() >= 3).await;
}

#[tokio::test]
async fn it_reconnects_after_the_tv_drops_it() {
    let tv = FakeTv::start().await;
    let handle = ready(&tv).await;
    assert_eq!(tv.accepted(), 1);
    tv.kick();
    until(&handle, |s| s.link == Link::Down).await;
    assert!(matches!(handle.press(KeyCode::Home), Err(Error::NotConnected)));
    until(&handle, |s| s.link == Link::Up && s.current_app.is_some()).await;
    assert_eq!(tv.accepted(), 2);
    handle.press(KeyCode::Home).unwrap();
    eventually("the key after reconnecting", || tv.keys() == [(KeyCode::Home, Direction::Short)]).await;
}

#[tokio::test]
async fn a_tv_that_forgot_us_reports_that_pairing_is_needed() {
    let tv = FakeTv::start().await;
    let handle = ready(&tv).await;
    tv.forget_clients();
    until(&handle, |s| s.link == Link::NotPaired).await;
}

#[tokio::test]
async fn an_unreachable_tv_is_not_connected() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let endpoint = androidtv::Endpoint::new("127.0.0.1").with_ports(port, port);
    let handle = connect(endpoint, &credentials()).unwrap();
    until(&handle, |s| s.link == Link::Down).await;
    assert!(matches!(handle.press(KeyCode::Home), Err(Error::NotConnected)));
}
