use bytes::Bytes;
use std::time::Duration;
use r3cui_bridge::link::testing::FakeBridge;
use r3cui_bridge::link::{connect_to, ErrorCode, Link, LinkEvent, SampleFormat, UiMsg};

async fn next(link: &mut Link) -> LinkEvent {
    tokio::time::timeout(Duration::from_secs(5), link.events.recv())
        .await
        .expect("no event")
        .expect("link ended")
}

#[tokio::test]
async fn a_session_arrives_and_the_outcome_goes_back() {
    let mut bridge = FakeBridge::start().await;
    let mut link = connect_to(bridge.addr());
    assert_eq!(next(&mut link).await, LinkEvent::Connected);
    bridge.wait_for_client().await;

    bridge.script_session(7, &[&[1, 2, 3, 4], &[5, 6]]);
    assert_eq!(
        next(&mut link).await,
        LinkEvent::Begin {
            session_id: 7,
            sample_rate: 16000,
            channels: 1,
            format: SampleFormat::I16
        }
    );
    assert_eq!(
        next(&mut link).await,
        LinkEvent::Audio {
            session_id: 7,
            pcm: Bytes::from_static(&[1, 2, 3, 4])
        }
    );
    assert_eq!(
        next(&mut link).await,
        LinkEvent::Audio {
            session_id: 7,
            pcm: Bytes::from_static(&[5, 6])
        }
    );
    assert_eq!(next(&mut link).await, LinkEvent::End { session_id: 7 });

    link.handle.send(UiMsg::Finished { session_id: 7 });
    assert_eq!(bridge.next_ui_msg().await, UiMsg::Finished { session_id: 7 });
    let error = UiMsg::Error {
        session_id: 7,
        code: ErrorCode::Timeout,
        message: "slow".into(),
    };
    link.handle.send(error.clone());
    assert_eq!(bridge.next_ui_msg().await, error);
}

#[tokio::test]
async fn audio_outside_a_session_is_dropped() {
    let mut bridge = FakeBridge::start().await;
    let mut link = connect_to(bridge.addr());
    assert_eq!(next(&mut link).await, LinkEvent::Connected);
    bridge.wait_for_client().await;
    bridge.send_audio([9u8, 9]);
    bridge.script_session(1, &[]);
    assert!(matches!(
        next(&mut link).await,
        LinkEvent::Begin { session_id: 1, .. }
    ));
}

#[tokio::test]
async fn reconnects_after_the_bridge_hangs_up() {
    let mut bridge = FakeBridge::start().await;
    let mut link = connect_to(bridge.addr());
    assert_eq!(next(&mut link).await, LinkEvent::Connected);
    bridge.wait_for_client().await;
    bridge.drop_client();
    assert_eq!(next(&mut link).await, LinkEvent::Disconnected);
    assert_eq!(next(&mut link).await, LinkEvent::Connected);
}

#[tokio::test]
async fn connects_when_the_bridge_starts_late() {
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = probe.local_addr().unwrap();
    drop(probe);
    let mut link = connect_to(addr.to_string());
    tokio::time::sleep(Duration::from_millis(100)).await;
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let accept = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let _ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    });
    assert_eq!(next(&mut link).await, LinkEvent::Connected);
    accept.abort();
}
