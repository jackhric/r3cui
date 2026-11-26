use futures_util::{SinkExt, StreamExt};
use prost::Message as _;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use voice_relay::proto::integration_message::Message as Voice;
use voice_relay::proto::{
    AudioConfiguration, IntegrationMessage, RemoteVoiceBegin, RemoteVoiceData, RemoteVoiceEnd,
    SampleFormat,
};
use voicelink::{connect_to, ErrorCode, Link, LinkEvent, UiMsg};

const WAIT: Duration = Duration::from_secs(5);

struct FakeCore {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    next_id: u64,
}

impl FakeCore {
    async fn connect(addr: &str) -> Self {
        let (ws, _) = connect_async(format!("ws://{addr}"))
            .await
            .expect("connect to the relay");
        Self { ws, next_id: 1 }
    }

    async fn frame(&mut self) -> Value {
        loop {
            let message = tokio::time::timeout(WAIT, self.ws.next())
                .await
                .expect("relay silent")
                .expect("relay hung up")
                .unwrap();
            match message {
                Message::Text(t) => return serde_json::from_str(&t).unwrap(),
                Message::Ping(_) | Message::Pong(_) => {}
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    async fn request(&mut self, msg: &str, data: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.ws
            .send(Message::Text(
                json!({"kind":"req","id":id,"msg":msg,"msg_data":data}).to_string(),
            ))
            .await
            .unwrap();
        id
    }

    async fn voice(&mut self, message: Voice) {
        self.ws
            .send(Message::Binary(
                IntegrationMessage {
                    message: Some(message),
                }
                .encode_to_vec(),
            ))
            .await
            .unwrap();
    }

    async fn voice_start(&mut self, session_id: u32) -> u64 {
        self.request(
            "entity_command",
            json!({"entity_type":"voice_assistant","entity_id":"voice","cmd_id":"voice_start","params":{"session_id":session_id,
                "audio_cfg":{"channels":1,"sample_rate":16000,"sample_format":"I16"}}}),
        )
        .await
    }
}

struct Rig {
    core_addr: String,
    link_addr: String,
}

async fn start() -> Rig {
    let integration = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let link = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rig = Rig {
        core_addr: integration.local_addr().unwrap().to_string(),
        link_addr: link.local_addr().unwrap().to_string(),
    };
    tokio::spawn(voice_relay::serve(integration, link));
    rig
}

async fn next_link(link: &mut Link) -> LinkEvent {
    tokio::time::timeout(WAIT, link.events.recv())
        .await
        .expect("no link event")
        .expect("link ended")
}

async fn core_with_handshake(rig: &Rig) -> FakeCore {
    let mut core = FakeCore::connect(&rig.core_addr).await;
    let auth = core.frame().await;
    assert_eq!(auth["msg"], "authentication");
    assert_eq!(auth["code"], 200);
    assert_eq!(auth["req_id"], 0);
    core
}

fn begin(session_id: u32) -> Voice {
    Voice::VoiceBegin(RemoteVoiceBegin {
        session_id,
        configuration: Some(AudioConfiguration {
            channels: 1,
            sample_rate: 16000,
            sample_format: SampleFormat::I16 as i32,
            format: 1,
        }),
    })
}

#[tokio::test]
async fn handshake_entities_and_subscription() {
    let rig = start().await;
    let mut core = core_with_handshake(&rig).await;

    let id = core.request("get_driver_version", Value::Null).await;
    let v = core.frame().await;
    assert_eq!(
        (v["msg"].as_str(), v["req_id"].as_u64()),
        (Some("driver_version"), Some(id))
    );
    assert_eq!(
        v["msg_data"]["version"]["driver"],
        env!("CARGO_PKG_VERSION")
    );

    core.request("get_driver_metadata", Value::Null).await;
    assert_eq!(core.frame().await["msg_data"]["driver_id"], "r3cui_voice");

    core.request("get_device_state", Value::Null).await;
    let v = core.frame().await;
    assert_eq!(
        (
            v["kind"].as_str(),
            v["msg"].as_str(),
            v["msg_data"]["state"].as_str()
        ),
        (Some("event"), Some("device_state"), Some("CONNECTED"))
    );

    core.request("get_available_entities", Value::Null).await;
    let v = core.frame().await;
    let entities = v["msg_data"]["available_entities"].as_array().unwrap();
    assert_eq!(entities.len(), 1);
    assert_eq!(
        (
            entities[0]["entity_id"].as_str(),
            entities[0]["entity_type"].as_str()
        ),
        (Some("voice"), Some("voice_assistant"))
    );
    assert_eq!(
        entities[0]["features"],
        json!(["transcription", "response_text"])
    );

    core.request("get_entity_states", Value::Null).await;
    assert_eq!(
        core.frame().await["msg_data"][0]["attributes"]["state"],
        "ON"
    );

    let id = core.request("subscribe_events", json!({})).await;
    let v = core.frame().await;
    assert_eq!(
        (v["msg"].as_str(), v["code"].as_u64(), v["req_id"].as_u64()),
        (Some("result"), Some(200), Some(id))
    );
    assert_eq!(core.frame().await["msg"], "device_state");
    assert_eq!(core.frame().await["msg"], "entity_change");

    core.ws
        .send(Message::Text(
            json!({"kind":"event","msg":"connect","cat":"DEVICE","msg_data":{}}).to_string(),
        ))
        .await
        .unwrap();
    assert_eq!(core.frame().await["msg"], "device_state");
    for standby in ["enter_standby", "exit_standby", "disconnect"] {
        core.ws
            .send(Message::Text(
                json!({"kind":"event","msg":standby,"msg_data":{}}).to_string(),
            ))
            .await
            .unwrap();
    }

    core.ws.send(Message::Ping(vec![1, 2])).await.unwrap();
    let pong = tokio::time::timeout(WAIT, async {
        loop {
            if let Some(Ok(Message::Pong(p))) = core.ws.next().await {
                return p;
            }
        }
    })
    .await
    .expect("no pong");
    assert_eq!(pong, vec![1, 2]);
}

#[tokio::test]
async fn a_session_reaches_r3cui_and_its_outcome_reaches_the_core() {
    let rig = start().await;
    let mut link = connect_to(&rig.link_addr);
    assert_eq!(next_link(&mut link).await, LinkEvent::Connected);
    let mut core = core_with_handshake(&rig).await;

    let id = core.voice_start(7).await;
    let v = core.frame().await;
    assert_eq!(
        (v["msg"].as_str(), v["code"].as_u64(), v["req_id"].as_u64()),
        (Some("result"), Some(200), Some(id))
    );
    assert_eq!(
        core.frame().await,
        json!({"kind":"event","msg":"assistant_event","cat":"ENTITY","msg_data":{"type":"ready","entity_id":"voice","session_id":7}})
    );

    core.voice(begin(7)).await;
    core.voice(Voice::VoiceData(RemoteVoiceData {
        session_id: 7,
        samples: vec![1, 2, 3, 4],
    }))
    .await;
    core.voice(Voice::VoiceData(RemoteVoiceData {
        session_id: 7,
        samples: vec![5, 6],
    }))
    .await;
    core.voice(Voice::VoiceEnd(RemoteVoiceEnd { session_id: 7 }))
        .await;

    assert_eq!(
        next_link(&mut link).await,
        LinkEvent::Begin {
            session_id: 7,
            sample_rate: 16000,
            channels: 1,
            format: voicelink::SampleFormat::I16
        }
    );
    assert_eq!(
        next_link(&mut link).await,
        LinkEvent::Audio {
            session_id: 7,
            pcm: vec![1, 2, 3, 4].into()
        }
    );
    assert_eq!(
        next_link(&mut link).await,
        LinkEvent::Audio {
            session_id: 7,
            pcm: vec![5, 6].into()
        }
    );
    assert_eq!(next_link(&mut link).await, LinkEvent::End { session_id: 7 });

    link.handle.send(UiMsg::Finished { session_id: 7 });
    assert_eq!(
        core.frame().await,
        json!({"kind":"event","msg":"assistant_event","cat":"ENTITY","msg_data":{"type":"finished","entity_id":"voice","session_id":7}})
    );
}

#[tokio::test]
async fn an_error_from_r3cui_becomes_an_assistant_error() {
    let rig = start().await;
    let mut link = connect_to(&rig.link_addr);
    assert_eq!(next_link(&mut link).await, LinkEvent::Connected);
    let mut core = core_with_handshake(&rig).await;
    core.voice_start(9).await;
    core.frame().await;
    core.frame().await;

    link.handle.send(UiMsg::Error {
        session_id: 9,
        code: ErrorCode::NoTextRecognized,
        message: "I did not understand".into(),
    });
    assert_eq!(
        core.frame().await,
        json!({"kind":"event","msg":"assistant_event","cat":"ENTITY","msg_data":{
            "type":"error","entity_id":"voice","session_id":9,
            "data":{"code":"NO_TEXT_RECOGNIZED","message":"I did not understand"}}})
    );
}

#[tokio::test]
async fn without_r3cui_the_start_is_answered_then_errors() {
    let rig = start().await;
    let mut core = core_with_handshake(&rig).await;
    core.voice_start(3).await;
    assert_eq!(core.frame().await["code"], 200);
    let v = core.frame().await;
    assert_eq!(v["msg"], "assistant_event");
    assert_eq!(v["msg_data"]["type"], "error");
    assert_eq!(v["msg_data"]["session_id"], 3);
    assert_eq!(
        v["msg_data"]["data"],
        json!({"code":"SERVICE_UNAVAILABLE","message":"r3cui is not listening"})
    );
}

#[tokio::test]
async fn a_lost_link_fails_the_running_session() {
    let rig = start().await;
    let mut link = connect_to(&rig.link_addr);
    assert_eq!(next_link(&mut link).await, LinkEvent::Connected);
    let mut core = core_with_handshake(&rig).await;
    core.voice_start(5).await;
    core.frame().await;
    core.frame().await;

    drop(link);
    let v = core.frame().await;
    assert_eq!(v["msg_data"]["data"]["code"], "SERVICE_UNAVAILABLE");
    assert_eq!(v["msg_data"]["session_id"], 5);
}

#[tokio::test]
async fn bad_commands_are_refused() {
    let rig = start().await;
    let mut core = core_with_handshake(&rig).await;

    let id = core
        .request(
            "entity_command",
            json!({"entity_id":"voice","cmd_id":"voice_start","params":{}}),
        )
        .await;
    let v = core.frame().await;
    assert_eq!(
        (v["code"].as_u64(), v["req_id"].as_u64()),
        (Some(400), Some(id))
    );

    core.request(
        "entity_command",
        json!({"entity_id":"nope","cmd_id":"voice_start","params":{"session_id":1}}),
    )
    .await;
    assert_eq!(core.frame().await["code"], 404);

    core.request(
        "entity_command",
        json!({"entity_id":"voice","cmd_id":"dance"}),
    )
    .await;
    assert_eq!(core.frame().await["code"], 501);

    core.request(
        "entity_command",
        json!({"entity_id":"voice","cmd_id":"voice_end"}),
    )
    .await;
    assert_eq!(core.frame().await["code"], 200);

    core.request("mystery", Value::Null).await;
    assert_eq!(core.frame().await["code"], 501);
}

#[tokio::test]
async fn garbage_binary_is_survived() {
    let rig = start().await;
    let mut link = connect_to(&rig.link_addr);
    assert_eq!(next_link(&mut link).await, LinkEvent::Connected);
    let mut core = core_with_handshake(&rig).await;
    core.ws
        .send(Message::Binary(vec![0xFF, 0xFF, 0xFF]))
        .await
        .unwrap();
    core.voice_start(2).await;
    assert_eq!(core.frame().await["code"], 200);
}
