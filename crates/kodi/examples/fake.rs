//! A stand-in Kodi for the desktop UI: `cargo run -p kodi --features testing --example fake -- 127.0.0.1:9090`.
//! Point the Kodi driver's address at it; its library has six films, and the d-pad walks it.

#[tokio::main]
async fn main() {
    let addr = std::env::args().nth(1).unwrap_or_else(|| "127.0.0.1:9090".into());
    let fake = kodi::testing::FakeKodi::start_on(&addr).await;
    println!("FakeKodi on ws://{}:{}/jsonrpc", fake.config().host, fake.config().ws_port);
    tokio::signal::ctrl_c().await.ok();
}
