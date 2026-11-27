//! Starting and ending a recording through the core: it records only for the voice assistant set on the remote,
//! which must be r3cui's bridge for the audio to reach r3cui.

use crate::coreapi::{Core, EntityId, VoiceStart};

async fn assistant(core: &Core) -> Option<EntityId> {
    match core.get_voice_control_cfg().await {
        Ok(cfg) if !cfg.microphone => tracing::warn!("voice: the remote's microphone is off"),
        Ok(cfg) => match cfg.voice_assistant.active {
            Some(a) => return Some(a.entity_id),
            None => tracing::warn!("voice: no voice assistant is set on the remote"),
        },
        Err(e) => tracing::warn!("voice: reading the voice settings: {e}"),
    }
    None
}

pub(crate) async fn start(core: &Core, session: u32) {
    if let Some(entity) = assistant(core).await {
        if let Err(e) = core.voice_start(&entity, &VoiceStart::new(session)).await {
            tracing::warn!("voice: starting session {session}: {e}");
        }
    }
}

pub(crate) async fn end(core: &Core, session: u32) {
    if let Some(entity) = assistant(core).await {
        if let Err(e) = core.voice_end(&entity, Some(session)).await {
            tracing::warn!("voice: ending session {session}: {e}");
        }
    }
}
