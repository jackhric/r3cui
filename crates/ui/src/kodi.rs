//! What the Kodi driver does not pass through the core: the backdrop art and the subtitle list.
//! Asked of Kodi's own JSON-RPC, on the web server its artwork already comes from. Blocking;
//! call from `spawn_blocking`.

use anyhow::Context;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Playing {
    /// Full image URL, or empty when the item has no backdrop.
    pub fanart: String,
    /// (Kodi's stream index, label)
    pub subtitles: Vec<(i64, String)>,
    /// Position in `subtitles` of the one shown; `None` when subtitles are off.
    pub subtitle: Option<usize>,
}

/// `http://:@host:8080/image/...` → `http://host:8080`
pub fn base(image_url: &str) -> Option<String> {
    let rest = image_url.strip_prefix("http://")?;
    let host = rest.split('/').next()?;
    let host = host.rsplit('@').next()?;
    (!host.is_empty()).then(|| format!("http://{host}"))
}

fn image(base: &str, art: &str) -> String {
    let mut out = format!("{base}/image/");
    for b in art.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn call(base: &str, method: &str, params: Value) -> anyhow::Result<Value> {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string();
    let reply = ureq::post(&format!("{base}/jsonrpc"))
        .timeout(Duration::from_secs(4))
        .set("Content-Type", "application/json")
        .send_string(&body)?
        .into_string()?;
    let v: Value = serde_json::from_str(&reply)?;
    if let Some(e) = v.get("error") {
        anyhow::bail!("{method}: {e}");
    }
    Ok(v.get("result").cloned().unwrap_or(Value::Null))
}

fn player(base: &str) -> anyhow::Result<i64> {
    let players = call(base, "Player.GetActivePlayers", json!({}))?;
    let list = players.as_array().context("no player list")?;
    let pick = list.iter().find(|p| p.get("type").and_then(Value::as_str) == Some("video")).or(list.first());
    pick.and_then(|p| p.get("playerid")).and_then(Value::as_i64).context("nothing playing")
}

pub fn playing(base: &str) -> anyhow::Result<Playing> {
    let id = player(base)?;
    let item = call(base, "Player.GetItem", json!({ "playerid": id, "properties": ["art", "fanart"] }))?;
    let item = item.get("item").cloned().unwrap_or(Value::Null);
    let art = |key: &str| item.pointer(&format!("/art/{key}")).and_then(Value::as_str).filter(|s| !s.is_empty());
    let fanart = art("fanart")
        .or_else(|| art("tvshow.fanart"))
        .or_else(|| item.get("fanart").and_then(Value::as_str).filter(|s| !s.is_empty()))
        .map(|a| image(base, a))
        .unwrap_or_default();

    let props = call(base, "Player.GetProperties", json!({ "playerid": id, "properties": ["subtitles", "currentsubtitle", "subtitleenabled"] }))?;
    let subtitles: Vec<(i64, String)> = props
        .get("subtitles")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| Some((s.get("index")?.as_i64()?, subtitle_label(s))))
        .collect();
    let enabled = props.get("subtitleenabled").and_then(Value::as_bool).unwrap_or(false);
    let current = props.pointer("/currentsubtitle/index").and_then(Value::as_i64);
    let subtitle = current.filter(|_| enabled).and_then(|c| subtitles.iter().position(|(i, _)| *i == c));
    Ok(Playing { fanart, subtitles, subtitle })
}

fn subtitle_label(s: &Value) -> String {
    let field = |k: &str| s.get(k).and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let (name, language) = (field("name"), field("language"));
    let mut label = match (name.is_empty(), language.is_empty()) {
        (false, false) if !name.to_lowercase().contains(&language.to_lowercase()) => format!("{name} ({language})"),
        (false, _) => name,
        (true, false) => language,
        (true, true) => "Unnamed".to_string(),
    };
    if s.get("isforced").and_then(Value::as_bool).unwrap_or(false) {
        label.push_str(" · Forced");
    }
    label
}

/// `None` turns subtitles off.
pub fn set_subtitle(base: &str, index: Option<i64>) -> anyhow::Result<()> {
    let id = player(base)?;
    let params = match index {
        Some(i) => json!({ "playerid": id, "subtitle": i, "enable": true }),
        None => json!({ "playerid": id, "subtitle": "off" }),
    };
    call(base, "Player.SetSubtitle", params).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_and_image_urls() {
        let url = "http://:@kodi.lan:8080/image/image%3A%2F%2Fx%2F";
        assert_eq!(base(url).as_deref(), Some("http://kodi.lan:8080"));
        assert_eq!(base("http://kodi.lan:8080/x").as_deref(), Some("http://kodi.lan:8080"));
        assert_eq!(image("http://k", "image://a b/"), "http://k/image/image%3A%2F%2Fa%20b%2F");
    }

    #[test]
    fn labels() {
        assert_eq!(subtitle_label(&json!({"name": "SDH", "language": "eng"})), "SDH (eng)");
        assert_eq!(subtitle_label(&json!({"name": "English", "language": "eng"})), "English");
        assert_eq!(subtitle_label(&json!({"name": "", "language": "fre", "isforced": true})), "fre · Forced");
    }
}
