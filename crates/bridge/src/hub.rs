use crate::api::{self, Assistant};
use crate::conn::Outbox;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use crate::link::{ErrorCode, BridgeMsg, SessionId, UiMsg};

pub const NOT_LISTENING: &str = "r3cui is not listening";
pub const LINK_LOST: &str = "r3cui disconnected";

struct Ui {
    id: u64,
    out: Outbox,
}

/// Where the two sides meet: the one r3cui on the link, and the core connection each running session answers to.
#[derive(Default)]
pub struct Hub {
    ui: Mutex<Option<Ui>>,
    sessions: Mutex<HashMap<SessionId, Outbox>>,
    next_ui: AtomicU64,
    heard: Mutex<Heard>,
}

/// What the core sent of the session's audio, for the log: whether the microphone reached the bridge at all.
#[derive(Default)]
struct Heard {
    frames: u64,
    bytes: u64,
    /// Of 16-bit samples, the loudest.
    peak: u16,
}

impl Hub {
    pub fn ui_connected(&self) -> bool {
        self.ui.lock().unwrap().is_some()
    }

    /// The new r3cui replaces the old one, which is hung up on.
    pub fn set_ui(&self, out: Outbox) -> u64 {
        let id = self.next_ui.fetch_add(1, Ordering::Relaxed);
        if let Some(old) = self.ui.lock().unwrap().replace(Ui { id, out }) {
            let _ = old.out.send(Message::Close(None));
        }
        id
    }

    /// Sessions that were running on this link cannot finish: the core is told.
    pub fn ui_gone(&self, id: u64) {
        {
            let mut ui = self.ui.lock().unwrap();
            if ui.as_ref().map(|u| u.id) != Some(id) {
                return;
            }
            *ui = None;
        }
        let sessions: Vec<_> = self.sessions.lock().unwrap().drain().collect();
        for (session_id, core) in sessions {
            let _ = core.send(api::assistant_event(
                session_id,
                Assistant::error(ErrorCode::ServiceUnavailable, LINK_LOST),
            ));
        }
    }

    pub fn open_session(&self, session_id: SessionId, core: Outbox) {
        self.sessions.lock().unwrap().insert(session_id, core);
    }

    pub fn audio_began(&self) {
        *self.heard.lock().unwrap() = Heard::default();
    }

    pub fn audio_ended(&self, session_id: SessionId) {
        let h = std::mem::take(&mut *self.heard.lock().unwrap());
        tracing::info!("session {session_id}: the core sent {} audio frames, {} bytes, peak {}", h.frames, h.bytes, h.peak);
    }

    pub fn to_ui(&self, msg: &BridgeMsg) {
        let text = serde_json::to_string(msg).expect("frame serialises");
        self.send_ui(Message::Text(text));
    }

    pub fn audio_to_ui(&self, pcm: Vec<u8>) {
        {
            let mut h = self.heard.lock().unwrap();
            h.frames += 1;
            h.bytes += pcm.len() as u64;
            let loudest = pcm.as_chunks::<2>().0.iter().map(|s| i16::from_le_bytes(*s).unsigned_abs()).max().unwrap_or(0);
            h.peak = h.peak.max(loudest);
        }
        self.send_ui(Message::Binary(pcm));
    }

    fn send_ui(&self, frame: Message) {
        if let Some(ui) = self.ui.lock().unwrap().as_ref() {
            let _ = ui.out.send(frame);
        }
    }

    pub fn ui_said(&self, msg: UiMsg) {
        let (session_id, event) = match msg {
            UiMsg::Finished { session_id } => (session_id, Assistant::Finished),
            UiMsg::Error {
                session_id,
                code,
                message,
            } => (session_id, Assistant::error(code, message)),
        };
        match self.sessions.lock().unwrap().remove(&session_id) {
            Some(core) => {
                let _ = core.send(api::assistant_event(session_id, event));
            }
            None => tracing::warn!("r3cui reported on session {session_id}, which is not running"),
        }
    }
}
