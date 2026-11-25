//! Reading the core's entities: their attributes, ids and names, and picking the ones to drive.

use serde_json::Value;

/// Env override first; otherwise prefer a Kodi media_player, then any matching media_player.
pub(crate) fn pick_entity(env: &str, entities: &[Value], accept: impl Fn(&Value) -> bool) -> Option<String> {
    if let Ok(id) = std::env::var(env) {
        return Some(id);
    }
    let players = || {
        entities
            .iter()
            .filter(|e| e.get("entity_type").and_then(Value::as_str) == Some("media_player"))
            .filter(|e| accept(e))
    };
    let is_kodi = |e: &&Value| {
        entity_id(e).to_lowercase().contains("kodi") || entity_name(e).to_lowercase().contains("kodi")
    };
    let pick = if env == "UC_SCREEN_ENTITY" {
        players().find(is_kodi)
    } else {
        // The receiver is the player that can switch inputs.
        players().find(|e| has_feature(e, "select_source") && !is_kodi(e))
    };
    pick.or_else(|| players().next()).map(|e| entity_id(e).to_string())
}
pub(crate) fn has_feature(e: &Value, feature: &str) -> bool {
    e.get("features")
        .and_then(Value::as_array)
        .is_some_and(|f| f.iter().any(|v| v.as_str() == Some(feature)))
}
pub(crate) fn attr_str(e: Option<&Value>, key: &str) -> String {
    e.and_then(|e| e.pointer(&format!("/attributes/{key}")))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}
pub(crate) fn attr_f64(e: Option<&Value>, key: &str) -> f64 {
    e.and_then(|e| e.pointer(&format!("/attributes/{key}"))).and_then(Value::as_f64).unwrap_or(0.0)
}
pub(crate) fn entity_id(e: &Value) -> &str {
    e.get("entity_id").and_then(Value::as_str).unwrap_or_default()
}
/// `name` may be a plain string or a `LanguageText` map (`{"en": "..."}`); fall back to the id.
pub(crate) fn entity_name(e: &Value) -> String {
    match e.get("name") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Object(m)) => m
            .get("en")
            .or_else(|| m.values().next())
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        _ => e.get("entity_id").and_then(Value::as_str).unwrap_or("?").to_string(),
    }
}
