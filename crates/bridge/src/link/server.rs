//! The bridge's end of the bridge link: r3cui connects here.

use crate::conn::{Conn, Frame};
use crate::hub::Hub;
use std::sync::Arc;
use tokio::net::TcpListener;
use super::UiMsg;

pub async fn serve(listener: TcpListener, hub: Arc<Hub>) {
    loop {
        let Ok((tcp, peer)) = listener.accept().await else {
            continue;
        };
        let hub = hub.clone();
        tokio::spawn(async move {
            let Some(mut conn) = Conn::accept(tcp).await else {
                return;
            };
            tracing::info!("r3cui connected from {peer}");
            let id = hub.set_ui(conn.out.clone());
            while let Some(frame) = conn.next().await {
                match frame {
                    Frame::Text(text) => match serde_json::from_str::<UiMsg>(&text) {
                        Ok(msg) => hub.ui_said(msg),
                        Err(e) => tracing::warn!("unreadable frame from r3cui {text:?}: {e}"),
                    },
                    Frame::Binary(_) => tracing::warn!("r3cui sent binary data"),
                }
            }
            tracing::info!("r3cui disconnected");
            hub.ui_gone(id);
        });
    }
}
