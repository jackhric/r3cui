//! r3cui's voice relay: a custom integration whose one `voice_assistant` entity receives the core's microphone audio
//! and hands it to r3cui over the voice link (`voicelink`). See `docs/voice.md`.

pub mod api;
mod conn;
mod hub;
mod intg;
mod link;
pub mod proto;

use hub::Hub;
use std::sync::Arc;
use tokio::net::TcpListener;

/// Serves the Integration-API on `integration` and the voice link on `link`, until the task is dropped.
pub async fn serve(integration: TcpListener, link: TcpListener) {
    let hub = Arc::new(Hub::default());
    tokio::join!(
        intg::serve(integration, hub.clone()),
        link::serve(link, hub)
    );
}
