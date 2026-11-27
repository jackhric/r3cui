#![cfg(feature = "testing")]

use homeassistant::testing::FakeHa;
use homeassistant::{EntityId, Error, HaConfig, HaHandle, HaService, HaState, Link, MediaPlayer};
use serde_json::json;
use wsclient::testing::until;

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

async fn until_state(handle: &HaHandle, what: &str, ok: impl Fn(&HaState) -> bool) -> HaState {
    until(&mut handle.state(), what, ok).await
}

fn id(s: &str) -> EntityId {
    s.into()
}

#[tokio::test]
async fn connects_and_sees_the_watched_entity() {
    let ha = fake().await;
    let handle = HaService::start(ha.config(), vec![id(RECEIVER)]);
    let s = until_state(&handle, "the receiver", |s| s.link == Link::Up && s.entities.contains_key(&id(RECEIVER))).await;
    assert_eq!(s.version.as_deref(), Some("2024.11.3"));
    assert_eq!(s.entities.len(), 1, "only what was asked for");
    let p = MediaPlayer(&s.entities[&id(RECEIVER)]);
    assert_eq!((p.is_on(), p.source(), p.volume_level()), (true, Some("KODI"), Some(0.56)));
}

#[tokio::test]
async fn a_bad_token_is_rejected() {
    let ha = fake().await;
    let handle = HaService::start(HaConfig::new(ha.config().url, "wrong"), vec![id(RECEIVER)]);
    let s = until_state(&handle, "rejection", |s| matches!(s.link, Link::Rejected(_))).await;
    assert_eq!(s.link, Link::Rejected("Invalid access token or password".into()));
    assert!(s.entities.is_empty());
    assert_eq!(handle.volume_set(&id(RECEIVER), 0.3).await, Err(Error::Closed));
}

#[tokio::test]
async fn an_unknown_scheme_is_unsupported() {
    let handle = HaService::start(HaConfig::new("ftp://ha.example.org", "t"), vec![]);
    let s = until_state(&handle, "the error", |s| matches!(s.link, Link::Down { .. })).await;
    assert!(matches!(s.link, Link::Down { error: Error::Unsupported(_), .. }));
}

#[tokio::test]
async fn a_service_call_changes_state_and_the_change_arrives() {
    let ha = fake().await;
    let handle = HaService::start(ha.config(), vec![id(RECEIVER)]);
    until_state(&handle, "up", |s| s.entities.contains_key(&id(RECEIVER))).await;

    handle.volume_set(&id(RECEIVER), 0.3).await.unwrap();
    until_state(&handle, "volume 0.3", |s| s.entities[&id(RECEIVER)].attr_f64("volume_level") == Some(0.3)).await;
    handle.volume_mute(&id(RECEIVER), true).await.unwrap();
    handle.select_source(&id(RECEIVER), "NET").await.unwrap();
    until_state(&handle, "mute and source", |s| {
        let e = &s.entities[&id(RECEIVER)];
        e.attr_bool("is_volume_muted") == Some(true) && e.attr_str("source") == Some("NET")
    })
    .await;
    handle.turn_off(&id(RECEIVER)).await.unwrap();
    until_state(&handle, "off", |s| s.entities[&id(RECEIVER)].state == "off").await;
    handle.turn_on(&id(RECEIVER)).await.unwrap();
    until_state(&handle, "on", |s| s.entities[&id(RECEIVER)].state == "on").await;

    handle.volume_up(&id(RECEIVER)).await.unwrap();
    handle.volume_up(&id(RECEIVER)).await.unwrap();
    handle.volume_down(&id(RECEIVER)).await.unwrap();
    until_state(&handle, "0.31", |s| s.entities[&id(RECEIVER)].attr_f64("volume_level") == Some(0.31)).await;

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
    until_state(&handle, "up", |s| s.entities.contains_key(&id(RECEIVER))).await;
    let mut attrs = receiver();
    attrs["volume_level"] = json!(0.7);
    attrs.as_object_mut().unwrap().remove("source");
    ha.set_entity(RECEIVER, "standby", attrs);
    let s = until_state(&handle, "standby", |s| s.entities[&id(RECEIVER)].state == "standby").await;
    let e = &s.entities[&id(RECEIVER)];
    assert_eq!((e.attr_f64("volume_level"), e.attr("source")), (Some(0.7), None));
}

#[tokio::test]
async fn failures_come_back_as_errors() {
    let ha = fake().await;
    let handle = HaService::start(ha.config(), vec![]);
    until_state(&handle, "up", |s| s.link == Link::Up).await;
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
    until_state(&handle, "up", |s| s.entities.contains_key(&id(RECEIVER))).await;
    assert_eq!(ha.connections(), 1);
    ha.kick();
    until_state(&handle, "down", |s| matches!(s.link, Link::Down { .. })).await;
    ha.set_entity(RECEIVER, "off", receiver());
    let s = until_state(&handle, "back", |s| s.link == Link::Up && s.entities.get(&id(RECEIVER)).is_some_and(|e| e.state == "off")).await;
    assert_eq!(s.entities.len(), 1);
    assert_eq!(ha.connections(), 2);
    handle.turn_on(&id(RECEIVER)).await.unwrap();
    until_state(&handle, "on again", |s| s.entities[&id(RECEIVER)].state == "on").await;
}

#[tokio::test]
async fn changing_the_watched_set_resubscribes() {
    let ha = fake().await;
    let handle = HaService::start(ha.config(), vec![id(RECEIVER)]);
    until_state(&handle, "receiver", |s| s.entities.contains_key(&id(RECEIVER))).await;

    handle.watch(vec![id("light.desk")]);
    let s = until_state(&handle, "the light only", |s| s.entities.len() == 1 && s.entities.contains_key(&id("light.desk"))).await;
    assert_eq!(s.entities[&id("light.desk")].state, "off");
    ha.set_entity("light.desk", "on", json!({"friendly_name": "Desk", "brightness": 255}));
    until_state(&handle, "light on", |s| s.entities.get(&id("light.desk")).is_some_and(|e| e.state == "on")).await;
    ha.set_entity(RECEIVER, "standby", receiver());
    ha.set_entity("light.desk", "off", json!({"friendly_name": "Desk"}));
    let s = until_state(&handle, "light off", |s| s.entities.get(&id("light.desk")).is_some_and(|e| e.state == "off")).await;
    assert!(!s.entities.contains_key(&id(RECEIVER)), "the old subscription is gone");

    handle.watch(vec![]);
    until_state(&handle, "nothing", |s| s.entities.is_empty()).await;
    assert_eq!(ha.connections(), 1, "no reconnect for a new watch set");
}

mod assist {
    use super::*;
    use homeassistant::protocol::assist::EventType;
    use homeassistant::{AssistEvent, EndStage, VoiceError};
    use std::time::Duration;
    use wsclient::testing::eventually;

    const TIMEOUT: Duration = Duration::from_secs(30);

    async fn connected() -> (FakeHa, HaHandle) {
        let ha = fake().await;
        let handle = HaService::start(ha.config(), vec![]);
        until_state(&handle, "the link", |s| s.link == Link::Up).await;
        (ha, handle)
    }

    async fn frames(ha: &FakeHa, count: usize) -> Vec<Vec<u8>> {
        eventually("the audio frames", || async { ha.audio_frames().len() >= count }).await;
        ha.audio_frames()
    }

    async fn next(run: &mut homeassistant::VoiceRun) -> AssistEvent {
        tokio::time::timeout(wsclient::testing::WAIT, run.next()).await.expect("an event in time").expect("the run is still going")
    }

    #[tokio::test]
    async fn lists_pipelines_and_the_preferred_one() {
        let (ha, handle) = connected().await;
        ha.set_pipelines(&[("p1", "Home Assistant", "en"), ("p2", "Cloud", "de")], Some("p2"));
        let got = handle.pipelines().await.unwrap();
        assert_eq!(got.list.iter().map(|p| (p.id.as_str(), p.name.as_str(), p.language.as_str())).collect::<Vec<_>>(), [("p1", "Home Assistant", "en"), ("p2", "Cloud", "de")]);
        assert_eq!(got.preferred.as_deref(), Some("p2"));
    }

    #[tokio::test]
    async fn a_run_asks_for_stt_to_the_end_stage_and_streams_audio_with_the_handler_id() {
        let (ha, handle) = connected().await;
        let run = handle.run_voice(Some("p1"), 16_000, EndStage::Intent, TIMEOUT).await.unwrap();
        let request = &ha.voice_runs()[0];
        assert_eq!(request["type"], "assist_pipeline/run");
        assert_eq!((request["start_stage"].as_str(), request["end_stage"].as_str(), request["pipeline"].as_str()), (Some("stt"), Some("intent"), Some("p1")));
        assert_eq!((request["input"]["sample_rate"].as_u64(), request["timeout"].as_u64()), (Some(16_000), Some(30)));

        let id = FakeHa::stt_binary_handler_id();
        run.send_audio(&[1, 2, 3, 4]).unwrap();
        run.send_audio(&[]).unwrap();
        run.send_audio(&[5, 6]).unwrap();
        run.end_audio().unwrap();
        assert_eq!(frames(&ha, 3).await, [vec![id, 1, 2, 3, 4], vec![id, 5, 6], vec![id]]);
    }

    #[tokio::test]
    async fn the_preferred_pipeline_and_tts_are_asked_for_by_leaving_out_the_id() {
        let (ha, handle) = connected().await;
        let _run = handle.run_voice(None, 16_000, EndStage::Tts, TIMEOUT).await.unwrap();
        let request = &ha.voice_runs()[0];
        assert_eq!(request["end_stage"], "tts");
        assert!(request.get("pipeline").is_none());
    }

    #[tokio::test]
    async fn a_full_run_reports_what_was_heard_the_answer_and_the_speech() {
        let (ha, handle) = connected().await;
        let mut run = handle.run_voice(None, 16_000, EndStage::Tts, TIMEOUT).await.unwrap();
        ha.run_event(EventType::SttStart, json!({}));
        ha.run_event(EventType::SttVadStart, json!({}));
        ha.run_event(EventType::SttVadEnd, json!({}));
        ha.run_event(EventType::SttEnd, json!({"stt_output": {"text": "turn on the desk light"}}));
        ha.run_event(EventType::IntentStart, json!({}));
        let answer = json!({"intent_output": {"response": {"response_type": "action_done", "speech": {"plain": {"speech": "Turned on the light"}}}}});
        ha.run_event(EventType::IntentEnd, answer);
        ha.run_event(EventType::TtsEnd, json!({"tts_output": {"url": "/api/tts_proxy/a.mp3", "mime_type": "audio/mpeg"}}));
        ha.run_event(EventType::RunEnd, json!({}));
        let mut got = Vec::new();
        while let Some(event) = tokio::time::timeout(wsclient::testing::WAIT, run.next()).await.expect("events in time") {
            got.push(event);
        }
        assert_eq!(
            got,
            [
                AssistEvent::Listening,
                AssistEvent::VadStart,
                AssistEvent::VadEnd,
                AssistEvent::Heard { text: "turn on the desk light".into() },
                AssistEvent::Answer { text: "Turned on the light".into(), success: true },
                AssistEvent::Speech { url: "/api/tts_proxy/a.mp3".into(), mime_type: "audio/mpeg".into() },
                AssistEvent::Finished,
            ]
        );
    }

    #[tokio::test]
    async fn run_start_before_the_result_still_starts_the_run() {
        let (ha, handle) = connected().await;
        ha.run_start_before_result(true);
        let run = handle.run_voice(None, 16_000, EndStage::Intent, TIMEOUT).await.unwrap();
        run.end_audio().unwrap();
        assert_eq!(frames(&ha, 1).await, [vec![FakeHa::stt_binary_handler_id()]]);
    }

    #[tokio::test]
    async fn an_answer_that_did_not_work_is_not_a_success() {
        let (ha, handle) = connected().await;
        let mut run = handle.run_voice(None, 16_000, EndStage::Intent, TIMEOUT).await.unwrap();
        let answer = json!({"intent_output": {"response": {"response_type": "error", "speech": {"plain": {"speech": "Sorry, I am not aware of any device"}}}}});
        ha.run_event(EventType::IntentEnd, answer);
        assert_eq!(next(&mut run).await, AssistEvent::Answer { text: "Sorry, I am not aware of any device".into(), success: false });
    }

    #[tokio::test]
    async fn an_error_event_fails_the_run_with_the_cores_code() {
        let (ha, handle) = connected().await;
        let mut run = handle.run_voice(None, 16_000, EndStage::Intent, TIMEOUT).await.unwrap();
        ha.run_event(EventType::Error, json!({"code": "stt-no-text-recognized", "message": "No text recognized"}));
        ha.run_event(EventType::RunEnd, json!({}));
        assert_eq!(next(&mut run).await, AssistEvent::Failed { code: VoiceError::NoTextRecognized, message: "No text recognized".into() });
        assert_eq!(run.next().await, None, "nothing after a failure");
    }

    #[tokio::test]
    async fn a_second_run_cancels_the_first_and_so_does_dropping() {
        let (ha, handle) = connected().await;
        let mut first = handle.run_voice(None, 16_000, EndStage::Intent, TIMEOUT).await.unwrap();
        let second = handle.run_voice(None, 16_000, EndStage::Intent, TIMEOUT).await.unwrap();
        eventually("the first run to be cancelled", || async { ha.voice_cancels() == 1 }).await;
        assert_eq!(first.next().await, None);
        drop(second);
        eventually("the second run to be cancelled", || async { ha.voice_cancels() == 2 }).await;
    }

    #[tokio::test]
    async fn losing_the_connection_ends_the_run() {
        let (ha, handle) = connected().await;
        let mut run = handle.run_voice(None, 16_000, EndStage::Intent, TIMEOUT).await.unwrap();
        ha.kick();
        assert_eq!(tokio::time::timeout(wsclient::testing::WAIT, run.next()).await, Ok(None));
    }

    #[tokio::test]
    async fn without_a_connection_nothing_starts() {
        let handle = HaService::start(HaConfig::new("https://ha.example.org", "t"), vec![]);
        assert_eq!(handle.pipelines().await, Err(Error::Closed));
        assert!(matches!(handle.run_voice(None, 16_000, EndStage::Intent, TIMEOUT).await, Err(Error::Closed)));
    }
}
