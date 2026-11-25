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

/// Seek `back` seconds behind where playback stands. This refills Kodi's read-ahead, which carries no subtitle
/// pictures for a track that was off when it was read; going back far enough to take in the start of the line
/// on show brings that line too.
#[cfg(feature = "preview")]
pub fn reload(base: &str, back: f64) -> anyhow::Result<()> {
    let id = player(base)?;
    let now = position(base)?;
    let to = (now - back).max(0.0);
    let ms = (to * 1000.0).round() as i64;
    let time = json!({ "hours": ms / 3_600_000, "minutes": ms / 60_000 % 60, "seconds": ms / 1000 % 60, "milliseconds": ms % 1000 });
    call(base, "Player.Seek", json!({ "playerid": id, "value": { "time": time } })).map(|_| ())
}

/// How far into what is playing Kodi is, in seconds, to the millisecond.
pub fn position(base: &str) -> anyhow::Result<f64> {
    let id = player(base)?;
    let at = call(base, "Player.GetProperties", json!({ "playerid": id, "properties": ["time"] }))?;
    let t = at.get("time").context("no time")?;
    let part = |k: &str| t.get(k).and_then(Value::as_f64).unwrap_or(0.0);
    Ok(part("hours") * 3600.0 + part("minutes") * 60.0 + part("seconds") + part("milliseconds") / 1000.0)
}

/// The title of what is playing, as Kodi itself names it.
pub fn title(base: &str) -> anyhow::Result<String> {
    let id = player(base)?;
    let item = call(base, "Player.GetItem", json!({ "playerid": id, "properties": ["title"] }))?;
    let item = item.get("item").context("no item")?;
    let title = item.get("title").and_then(Value::as_str).filter(|t| !t.is_empty());
    let title = title.or_else(|| item.get("label").and_then(Value::as_str)).unwrap_or("");
    anyhow::ensure!(!title.is_empty(), "untitled");
    Ok(title.to_string())
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

/// Kodi reports subtitle languages as ISO 639-2 codes ("eng", "ger" or "deu"); the picker shows the language's name.
fn language_name(code: &str) -> Option<&'static str> {
    Some(match code.to_ascii_lowercase().as_str() {
        "eng" | "en" => "English",
        "ara" | "ar" => "Arabic",
        "chi" | "zho" | "zh" => "Chinese",
        "hrv" | "hr" => "Croatian",
        "cze" | "ces" | "cs" => "Czech",
        "dan" | "da" => "Danish",
        "dut" | "nld" | "nl" => "Dutch",
        "fin" | "fi" => "Finnish",
        "fre" | "fra" | "fr" => "French",
        "ger" | "deu" | "de" => "German",
        "gre" | "ell" | "el" => "Greek",
        "heb" | "he" => "Hebrew",
        "hin" | "hi" => "Hindi",
        "hun" | "hu" => "Hungarian",
        "ice" | "isl" | "is" => "Icelandic",
        "ind" | "id" => "Indonesian",
        "ita" | "it" => "Italian",
        "jpn" | "ja" => "Japanese",
        "kor" | "ko" => "Korean",
        "may" | "msa" | "ms" => "Malay",
        "nor" | "nob" | "no" | "nb" => "Norwegian",
        "pol" | "pl" => "Polish",
        "por" | "pt" => "Portuguese",
        "rum" | "ron" | "ro" => "Romanian",
        "rus" | "ru" => "Russian",
        "slo" | "slk" | "sk" => "Slovak",
        "slv" | "sl" => "Slovenian",
        "spa" | "es" => "Spanish",
        "srp" | "sr" => "Serbian",
        "swe" | "sv" => "Swedish",
        "tha" | "th" => "Thai",
        "tur" | "tr" => "Turkish",
        "ukr" | "uk" => "Ukrainian",
        "vie" | "vi" => "Vietnamese",
        "bul" | "bg" => "Bulgarian",
        "cat" | "ca" => "Catalan",
        "est" | "et" => "Estonian",
        "lav" | "lv" => "Latvian",
        "lit" | "lt" => "Lithuanian",
        "per" | "fas" | "fa" => "Persian",
        _ => return None,
    })
}

fn subtitle_label(s: &Value) -> String {
    let field = |k: &str| s.get(k).and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let (name, code) = (field("name"), field("language"));
    let language = language_name(&code).map(str::to_string).unwrap_or(code);
    let mut label = match (name.is_empty(), language.is_empty()) {
        (false, false) if !name.to_lowercase().contains(&language.to_lowercase()) => format!("{language} ({name})"),
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
        assert_eq!(subtitle_label(&json!({"name": "SDH", "language": "eng"})), "English (SDH)");
        assert_eq!(subtitle_label(&json!({"name": "English", "language": "eng"})), "English");
        assert_eq!(subtitle_label(&json!({"name": "", "language": "fre", "isforced": true})), "French · Forced");
        assert_eq!(subtitle_label(&json!({"name": "Simplified", "language": "chi"})), "Chinese (Simplified)");
        assert_eq!(subtitle_label(&json!({"name": "", "language": "ara"})), "Arabic");
        assert_eq!(subtitle_label(&json!({"name": "", "language": "xyz"})), "xyz");
    }
}

/// What Kodi's own window is showing, for the remote's library wall.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Browsing {
    /// `movies`, `tvshows`, … or empty outside a library list.
    pub content: String,
    pub path: String,
    pub sort: String,
    pub order: String,
    /// Library id of the highlighted item.
    pub focus: i64,
    /// The highlight's column and row in Kodi's panel, and its place among the cells on show; -1 when Kodi
    /// gives none.
    pub column: i64,
    pub row: i64,
    pub position: i64,
}

pub fn browsing(base: &str) -> anyhow::Result<Browsing> {
    let labels = [
        "Container.Content", "Container.FolderPath", "Container.SortMethod", "Container.SortOrder", "ListItem.DBID",
        "Container.Column", "Container.Row", "Container.Position",
    ];
    let v = call(base, "XBMC.GetInfoLabels", json!({ "labels": labels }))?;
    let get = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let num = |k: &str| get(k).parse().unwrap_or(-1);
    Ok(Browsing {
        content: get("Container.Content"),
        path: get("Container.FolderPath"),
        sort: get("Container.SortMethod"),
        order: get("Container.SortOrder"),
        focus: num("ListItem.DBID"),
        column: num("Container.Column"),
        row: num("Container.Row"),
        position: num("Container.Position"),
    })
}

/// The list behind `b`, in the order Kodi shows it, with each poster as a full image URL.
pub fn list(base: &str, b: &Browsing) -> anyhow::Result<Vec<Value>> {
    let shows = b.content == "tvshows";
    let fields: &[&str] = if shows {
        &["title", "year", "art", "rating", "genre", "plot", "season", "episode", "watchedepisodes", "mpaa"]
    } else {
        &["title", "year", "art", "runtime", "rating", "genre", "plot", "resume", "playcount", "mpaa"]
    };
    // Kodi reports its sort by the name shown in its menu.
    let method = match b.sort.to_lowercase().as_str() {
        "year" => "year",
        "date added" => "dateadded",
        "rating" | "user rating" => "rating",
        "last played" => "lastplayed",
        "play count" => "playcount",
        "duration" | "runtime" => "runtime",
        "mpaa" => "mpaa",
        // Kodi's own Title sort goes by each item's sort title ("bronx tale" for "A Bronx Tale").
        _ => "sorttitle",
    };
    let order = if b.order.eq_ignore_ascii_case("descending") { "descending" } else { "ascending" };
    let params = json!({
        "directory": b.path, "media": "video", "properties": fields,
        "sort": { "method": method, "order": order, "ignorearticle": true },
    });
    let v = call(base, "Files.GetDirectory", params)?;
    let mut files = v.get("files").and_then(Value::as_array).cloned().unwrap_or_default();
    for f in &mut files {
        let art = f.pointer("/art/poster").or_else(|| f.pointer("/art/thumb")).and_then(Value::as_str).unwrap_or("").to_string();
        f["poster"] = Value::String(if art.is_empty() { art } else { image(base, &smaller(&art, 136, 204)) });
    }
    Ok(files)
}

/// Kodi's art for Jellyfin items points at the full-size original. Asking Jellyfin for a resized
/// copy instead, still through Kodi's image cache, cuts a poster from ~40 KB to ~5 KB. Other art
/// is left alone.
pub fn smaller(art: &str, w: u32, h: u32) -> String {
    let Some(inner) = art.strip_prefix("image://").map(|a| a.trim_end_matches('/')) else { return art.to_string() };
    let url = decode(inner);
    if !url.contains("/Images/") || !url.contains("Format=original") {
        return art.to_string();
    }
    let url = url.replace("Format=original", &format!("fillWidth={w}&fillHeight={h}&quality=85"));
    let mut out = String::from("image://");
    for b in url.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02x}"));
        }
    }
    out.push('/');
    out
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod small_art_tests {
    use super::*;

    #[test]
    fn jellyfin_posters_are_asked_for_small() {
        let art = "image://https%3a%2f%2fmedia.example.com%2fItems%2fabc%2fImages%2fPrimary%2f0%3fFormat%3doriginal%26Tag%3d123/";
        assert_eq!(
            smaller(art, 136, 204),
            "image://https%3a%2f%2fmedia.example.com%2fItems%2fabc%2fImages%2fPrimary%2f0%3ffillWidth%3d136%26fillHeight%3d204%26quality%3d85%26Tag%3d123/"
        );
        assert_eq!(smaller("image://DefaultVideo.png/", 136, 204), "image://DefaultVideo.png/");
    }
}
