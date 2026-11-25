#![cfg(feature = "testing")]

use homeassistant::testing::FakeHa;
use homeassistant::{EntityId, Error, HaConfig, HaHandle, HaService, HaState, Link, MediaPlayer};
use serde_json::json;
use std::time::Duration;

const RECEIVER: &str = "media_player.theater_receiver";
const TOKEN: &str = "llat-secret";

fn receiver() -> serde_json::Value {
    json!({
        "source_list": ["ANDROID TV", "Bluetooth", "HEOS Music", "KODI", "NET", "PLAYSTATION 5", "Source", "XBOX 360"],
        "volume_level": 0.56, "is_volume_muted": false, "source": "KODI", "device_class": "receiver",
        "friendly_name": "Theater Receiver", "supported_features": 69004
    })
}

async fn fake() -> FakeHa {
    let ha = FakeHa::start(TOKEN).await;
    ha.set_entity(RECEIVER, "on", receiver());
    ha.set_entity("light.desk", "off", json!({"friendly_name": "Desk"}));
    ha
}

async fn until(handle: &HaHandle, what: &str, ok: impl Fn(&HaState) -> bool) -> HaState {
    let mut rx = handle.state();
    let found = tokio::time::timeout(Duration::from_secs(5), rx.wait_for(|s| ok(s))).await.map(|r| r.map(|s| s.clone()));
    match found {
        Ok(Ok(s)) => s,
        _ => panic!("never saw {what}; last state {:?}", rx.borrow()),
    }
}

fn id(s: &str) -> EntityId {
    s.into()
}

#[tokio::test]
async fn connects_and_sees_the_watched_entity() {
    let ha = fake().await;
    let handle = HaService::start(ha.config(), vec![id(RECEIVER)]);
    let s = until(&handle, "the receiver", |s| s.link == Link::Up && s.entities.contains_key(&id(RECEIVER))).await;
    assert_eq!(s.version.as_deref(), Some("2024.11.3"));
    assert_eq!(s.entities.len(), 1, "only what was asked for");
    let p = MediaPlayer(&s.entities[&id(RECEIVER)]);
    assert_eq!((p.is_on(), p.source(), p.volume_level()), (true, Some("KODI"), Some(0.56)));
}

#[tokio::test]
async fn a_bad_token_is_rejected() {
    let ha = fake().await;
    let handle = HaService::start(HaConfig::new(ha.config().url, "wrong"), vec![id(RECEIVER)]);
    let s = until(&handle, "rejection", |s| matches!(s.link, Link::Rejected(_))).await;
    assert_eq!(s.link, Link::Rejected("Invalid access token or password".into()));
    assert!(s.entities.is_empty());
    assert_eq!(handle.volume_set(&id(RECEIVER), 0.3).await, Err(Error::Closed));
}

#[tokio::test]
async fn https_is_not_supported_yet() {
    let handle = HaService::start(HaConfig::new("https://ha.example.org", "t"), vec![]);
    let s = until(&handle, "the error", |s| matches!(s.link, Link::Down { .. })).await;
    assert!(matches!(s.link, Link::Down { error: Error::Unsupported(_), .. }));
}

#[tokio::test]
async fn a_service_call_changes_state_and_the_change_arrives() {
    let ha = fake().await;
    let handle = HaService::start(ha.config(), vec![id(RECEIVER)]);
    until(&handle, "up", |s| s.entities.contains_key(&id(RECEIVER))).await;

    handle.volume_set(&id(RECEIVER), 0.3).await.unwrap();
    until(&handle, "volume 0.3", |s| s.entities[&id(RECEIVER)].attr_f64("volume_level") == Some(0.3)).await;
    handle.volume_mute(&id(RECEIVER), true).await.unwrap();
    handle.select_source(&id(RECEIVER), "NET").await.unwrap();
    until(&handle, "mute and source", |s| {
        let e = &s.entities[&id(RECEIVER)];
        e.attr_bool("is_volume_muted") == Some(true) && e.attr_str("source") == Some("NET")
    })
    .await;
    handle.turn_off(&id(RECEIVER)).await.unwrap();
    until(&handle, "off", |s| s.entities[&id(RECEIVER)].state == "off").await;
    handle.turn_on(&id(RECEIVER)).await.unwrap();
    until(&handle, "on", |s| s.entities[&id(RECEIVER)].state == "on").await;

    handle.volume_up(&id(RECEIVER)).await.unwrap();
    handle.volume_up(&id(RECEIVER)).await.unwrap();
    handle.volume_down(&id(RECEIVER)).await.unwrap();
    until(&handle, "0.31", |s| s.entities[&id(RECEIVER)].attr_f64("volume_level") == Some(0.31)).await;

    let calls: Vec<_> = ha.calls().into_iter().map(|(d, s, _)| format!("{d}.{s}")).collect();
    assert_eq!(
        calls,
        ["volume_set", "volume_mute", "select_source", "turn_off", "turn_on", "volume_up", "volume_up", "volume_down"].map(|s| format!("media_player.{s}"))
    );
    assert_eq!(ha.calls()[0].2, json!({"volume_level": 0.3}));
}

#[tokio::test]
async fn changes_made_elsewhere_arrive_too() {
    let ha = fake().await;
    let handle = HaService::start(ha.config(), vec![id(RECEIVER)]);
    until(&handle, "up", |s| s.entities.contains_key(&id(RECEIVER))).await;
    let mut attrs = receiver();
    attrs["volume_level"] = json!(0.7);
    attrs.as_object_mut().unwrap().remove("source");
    ha.set_entity(RECEIVER, "standby", attrs);
    let s = until(&handle, "standby", |s| s.entities[&id(RECEIVER)].state == "standby").await;
    let e = &s.entities[&id(RECEIVER)];
    assert_eq!((e.attr_f64("volume_level"), e.attr("source")), (Some(0.7), None));
}

#[tokio::test]
async fn failures_come_back_as_errors() {
    let ha = fake().await;
    let handle = HaService::start(ha.config(), vec![]);
    until(&handle, "up", |s| s.link == Link::Up).await;
    let all = handle.states().await.unwrap();
    assert_eq!(all.len(), 2);
    assert!(all.iter().any(|(i, e)| i.domain() == "media_player" && e.state == "on"));
    let err = handle.call_service("media_player", "volume_set", &id(RECEIVER), json!({})).await;
    assert_eq!(err, Ok(()), "the fake accepts any service");
}

#[tokio::test]
async fn reconnects_after_the_connection_drops() {
    let ha = fake().await;
    let handle = HaService::start(ha.config(), vec![id(RECEIVER)]);
    until(&handle, "up", |s| s.entities.contains_key(&id(RECEIVER))).await;
    assert_eq!(ha.connections(), 1);
    ha.kick();
    until(&handle, "down", |s| matches!(s.link, Link::Down { .. })).await;
    ha.set_entity(RECEIVER, "off", receiver());
    let s = until(&handle, "back", |s| s.link == Link::Up && s.entities.get(&id(RECEIVER)).is_some_and(|e| e.state == "off")).await;
    assert_eq!(s.entities.len(), 1);
    assert_eq!(ha.connections(), 2);
    handle.turn_on(&id(RECEIVER)).await.unwrap();
    until(&handle, "on again", |s| s.entities[&id(RECEIVER)].state == "on").await;
}

#[tokio::test]
async fn changing_the_watched_set_resubscribes() {
    let ha = fake().await;
    let handle = HaService::start(ha.config(), vec![id(RECEIVER)]);
    until(&handle, "receiver", |s| s.entities.contains_key(&id(RECEIVER))).await;

    handle.watch(vec![id("light.desk")]);
    let s = until(&handle, "the light only", |s| s.entities.len() == 1 && s.entities.contains_key(&id("light.desk"))).await;
    assert_eq!(s.entities[&id("light.desk")].state, "off");
    ha.set_entity("light.desk", "on", json!({"friendly_name": "Desk", "brightness": 255}));
    until(&handle, "light on", |s| s.entities.get(&id("light.desk")).is_some_and(|e| e.state == "on")).await;
    ha.set_entity(RECEIVER, "standby", receiver());
    ha.set_entity("light.desk", "off", json!({"friendly_name": "Desk"}));
    let s = until(&handle, "light off", |s| s.entities.get(&id("light.desk")).is_some_and(|e| e.state == "off")).await;
    assert!(!s.entities.contains_key(&id(RECEIVER)), "the old subscription is gone");

    handle.watch(vec![]);
    until(&handle, "nothing", |s| s.entities.is_empty()).await;
    assert_eq!(ha.connections(), 1, "no reconnect for a new watch set");
}
