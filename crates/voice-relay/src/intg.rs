//! The Integration-API server the remote's core connects to.

use crate::api::{self, cmd, code, msg, Assistant, DeviceState, EntityCommand, Incoming};
use crate::conn::{Conn, Frame, Outbox};
use crate::hub::{Hub, NOT_LISTENING};
use crate::proto::integration_message::Message as Voice;
use crate::proto::{self, IntegrationMessage};
use prost::Message as _;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::net::TcpListener;
use voicelink::{ErrorCode, RelayMsg, SampleFormat, UiMsg};

const DEFAULT_SAMPLE_RATE: u32 = 16000;
const DEFAULT_CHANNELS: u16 = 1;

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
            tracing::info!("core connected from {peer}");
            let _ = conn.out.send(api::response(
                0,
                code::OK,
                msg::AUTHENTICATION,
                Some(api::driver_version()),
            ));
            while let Some(frame) = conn.next().await {
                match frame {
                    Frame::Text(text) => handle_text(&hub, &conn.out, &text),
                    Frame::Binary(bytes) => handle_binary(&hub, &bytes),
                }
            }
            tracing::info!("core disconnected");
        });
    }
}

fn handle_text(hub: &Hub, out: &Outbox, text: &str) {
    let incoming = match serde_json::from_str::<Incoming>(text) {
        Ok(i) => i,
        Err(e) => return tracing::warn!("unreadable frame from the core {text:?}: {e}"),
    };
    let id = incoming.id;
    let send = |m| {
        let _ = out.send(m);
    };
    if incoming.kind == api::kind::EVENT {
        return match incoming.msg.as_str() {
            msg::CONNECT => send(api::device_state(DeviceState::Connected)),
            msg::DISCONNECT | msg::ENTER_STANDBY | msg::EXIT_STANDBY => {
                tracing::debug!("core event {}", incoming.msg)
            }
            other => tracing::debug!("ignored core event {other}"),
        };
    }
    if incoming.kind != api::kind::REQ {
        return;
    }
    match incoming.msg.as_str() {
        msg::GET_DRIVER_VERSION => send(api::response(
            id,
            code::OK,
            msg::DRIVER_VERSION,
            Some(api::driver_version()),
        )),
        msg::GET_DRIVER_METADATA => send(api::response(
            id,
            code::OK,
            msg::DRIVER_METADATA,
            Some(api::driver_metadata()),
        )),
        msg::GET_DEVICE_STATE => send(api::device_state(DeviceState::Connected)),
        msg::GET_AVAILABLE_ENTITIES => send(api::response(
            id,
            code::OK,
            msg::AVAILABLE_ENTITIES,
            Some(json!({ "available_entities": [api::entity()] })),
        )),
        msg::GET_ENTITY_STATES => send(api::response(
            id,
            code::OK,
            msg::ENTITY_STATES,
            Some(api::entity_states()),
        )),
        msg::SUBSCRIBE_EVENTS => {
            send(api::result(id, code::OK));
            send(api::device_state(DeviceState::Connected));
            send(api::entity_change());
        }
        msg::UNSUBSCRIBE_EVENTS => send(api::result(id, code::OK)),
        msg::SETUP_DRIVER => {
            send(api::result(id, code::OK));
            send(api::setup_finished());
        }
        msg::ENTITY_COMMAND => entity_command(hub, out, id, incoming.msg_data),
        other => {
            tracing::debug!("unsupported request {other}");
            send(api::result(id, code::NOT_IMPLEMENTED));
        }
    }
}

fn entity_command(hub: &Hub, out: &Outbox, req_id: u64, data: Value) {
    let reply = |c| {
        let _ = out.send(api::result(req_id, c));
    };
    let Ok(command) = serde_json::from_value::<EntityCommand>(data) else {
        return reply(code::BAD_REQUEST);
    };
    if command.entity_id != api::ENTITY_ID {
        return reply(code::NOT_FOUND);
    }
    match command.cmd_id.as_str() {
        cmd::VOICE_START => {
            let Some(session_id) = command
                .params
                .get("session_id")
                .and_then(Value::as_u64)
                .and_then(|s| u32::try_from(s).ok())
                .filter(|s| *s > 0)
            else {
                return reply(code::BAD_REQUEST);
            };
            tracing::info!("voice_start session {session_id}, r3cui {}", if hub.ui_connected() { "listening" } else { "not listening" });
            // Ready even with no r3cui on the link: the audio still comes, and is logged, and the session fails at its
            // end instead, so a missing stream and a missing r3cui can be told apart.
            hub.open_session(session_id, out.clone());
            reply(code::OK);
            let _ = out.send(api::assistant_event(session_id, Assistant::Ready));
        }
        cmd::VOICE_END => {
            tracing::info!("voice_end");
            reply(code::OK)
        }
        _ => reply(code::NOT_IMPLEMENTED),
    }
}

fn handle_binary(hub: &Hub, bytes: &[u8]) {
    let frame = match IntegrationMessage::decode(bytes) {
        Ok(IntegrationMessage {
            message: Some(frame),
        }) => frame,
        Ok(_) => return tracing::warn!("empty voice frame"),
        Err(e) => return tracing::warn!("undecodable voice frame: {e}"),
    };
    match frame {
        Voice::VoiceBegin(begin) => {
            let cfg = begin.configuration.unwrap_or_default();
            tracing::info!("session {}: audio begins, {} Hz, {} channels, format {}", begin.session_id, cfg.sample_rate, cfg.channels, cfg.sample_format);
            hub.audio_began();
            hub.to_ui(&RelayMsg::Begin {
                session_id: begin.session_id,
                sample_rate: if cfg.sample_rate == 0 {
                    DEFAULT_SAMPLE_RATE
                } else {
                    cfg.sample_rate
                },
                channels: if cfg.channels == 0 {
                    DEFAULT_CHANNELS
                } else {
                    cfg.channels.min(u16::MAX as u32) as u16
                },
                format: sample_format(cfg.sample_format),
            });
        }
        Voice::VoiceData(data) => hub.audio_to_ui(data.samples),
        Voice::VoiceEnd(end) => {
            hub.audio_ended(end.session_id);
            if !hub.ui_connected() {
                hub.ui_said(UiMsg::Error { session_id: end.session_id, code: ErrorCode::ServiceUnavailable, message: NOT_LISTENING.into() });
            }
            hub.to_ui(&RelayMsg::End {
                session_id: end.session_id,
            })
        }
    }
}

fn sample_format(raw: i32) -> SampleFormat {
    match proto::SampleFormat::try_from(raw) {
        Ok(proto::SampleFormat::I8) => SampleFormat::I8,
        Ok(proto::SampleFormat::I32) => SampleFormat::I32,
        Ok(proto::SampleFormat::U8) => SampleFormat::U8,
        Ok(proto::SampleFormat::U16) => SampleFormat::U16,
        Ok(proto::SampleFormat::U32) => SampleFormat::U32,
        Ok(proto::SampleFormat::F32) => SampleFormat::F32,
        Ok(proto::SampleFormat::I16) | Ok(proto::SampleFormat::Unknown) | Err(_) => {
            SampleFormat::I16
        }
    }
}
