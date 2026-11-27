//! r3cui's Unfolded Circle integration: it bridges remote functions only integrations can reach (today the
//! microphone, through its `voice_assistant` entity) to r3cui over the bridge link (`link`). See `docs/voice.md`.

pub mod api;
mod conn;
mod hub;
mod intg;
pub mod link;
pub mod proto;

use hub::Hub;
use std::sync::Arc;
use tokio::net::TcpListener;

/// Serves the Integration-API on `integration` and the bridge link on `link`, until the task is dropped.
pub async fn serve(integration: TcpListener, link: TcpListener) {
    let hub = Arc::new(Hub::default());
    tokio::join!(
        intg::serve(integration, hub.clone()),
        link::serve(link, hub)
    );
}
