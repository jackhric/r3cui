//! Follow entities on a Home Assistant and print them as they change; read only, it calls no services:
//! `HA_URL=http://homeassistant.local:8123 HA_TOKEN=... cargo run -p homeassistant --example ha-watch -- media_player.theater_receiver`.

use homeassistant::{EntityId, HaConfig, HaService};

#[tokio::main]
async fn main() {
    let url = std::env::var("HA_URL").expect("HA_URL, like http://homeassistant.local:8123");
    let token = std::env::var("HA_TOKEN").expect("HA_TOKEN, a long-lived access token");
    let ids: Vec<EntityId> = std::env::args().skip(1).map(EntityId::new).collect();
    let handle = HaService::start(HaConfig::new(url, token), ids);
    let mut state = handle.state();
    let start = tokio::time::Instant::now();
    while state.changed().await.is_ok() {
        let s = state.borrow().clone();
        println!("{:>7.2}s {:?} {}", start.elapsed().as_secs_f32(), s.link, s.version.as_deref().unwrap_or(""));
        for (id, e) in &s.entities {
            println!("          {id} = {} {}", e.state, serde_json::Value::Object(e.attributes.clone()));
        }
    }
}
