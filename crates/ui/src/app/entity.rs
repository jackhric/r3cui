//! Picking the core's entities to drive, and reading attributes from one that may not be there.

use coreapi::{Attr, Entity, EntityType};

/// Env override first; otherwise prefer a Kodi media_player, then any matching media_player.
pub(crate) fn pick_entity(env: &str, entities: &[Entity], accept: impl Fn(&Entity) -> bool) -> Option<String> {
    if let Ok(id) = std::env::var(env) {
        return Some(id);
    }
    let players = || entities.iter().filter(|e| e.entity_type == EntityType::MediaPlayer).filter(|e| accept(e));
    let is_kodi = |e: &&Entity| e.entity_id.to_lowercase().contains("kodi") || e.name().to_lowercase().contains("kodi");
    let pick = if env == "UC_SCREEN_ENTITY" {
        players().find(is_kodi)
    } else {
        // The receiver is the player that can switch inputs.
        players().find(|e| e.has_feature("select_source") && !is_kodi(e))
    };
    pick.or_else(|| players().next()).map(|e| e.entity_id.clone())
}

pub(crate) fn text(e: Option<&Entity>, a: Attr) -> &str {
    e.map_or("", |e| e.text(a))
}

pub(crate) fn number(e: Option<&Entity>, a: Attr) -> f64 {
    e.and_then(|e| e.number(a)).unwrap_or(0.0)
}

pub(crate) fn list(e: Option<&Entity>, a: Attr) -> Vec<String> {
    e.map_or_else(Vec::new, |e| e.list(a))
}
