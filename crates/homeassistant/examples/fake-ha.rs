//! A stand-in Home Assistant for the desktop UI:
//! `cargo run -p homeassistant --features testing --example fake-ha -- 127.0.0.1:8124 token`.
//! It has a receiver (`media_player.theater_receiver`) whose volume, mute, input and power follow the remote.

use serde_json::json;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let addr = args.next().unwrap_or_else(|| "127.0.0.1:8124".into());
    let token = args.next().unwrap_or_else(|| "token".into());
    let fake = homeassistant::testing::FakeHa::start_on(&addr, &token).await;
    fake.set_entity(
        "media_player.theater_receiver",
        "on",
        json!({
            "friendly_name": "Theater Receiver", "device_class": "receiver", "supported_features": 69004,
            "source_list": ["ANDROID TV", "Bluetooth", "HEOS Music", "KODI", "NET", "PLAYSTATION 5", "XBOX 360"],
            "source": "KODI", "volume_level": 0.56, "is_volume_muted": false,
        }),
    );
    fake.set_entity("sensor.theater_receiver_volume_limit", "72", json!({ "friendly_name": "Receiver Volume Limit" }));
    fake.set_entity("light.theater_remote_keys", "off", json!({ "friendly_name": "Remote Keys" }));
    println!("FakeHa on {} (token {token:?})", fake.config().url);
    tokio::signal::ctrl_c().await.ok();
}
