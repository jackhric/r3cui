//! Text for the screen: times, volume, audio codecs and channel layouts, input and codec logos.

use crate::{AppWindow, Codecs, Logos};
use slint::{ComponentHandle, SharedString};

pub(crate) fn band_label(band: &str) -> Option<&'static str> {
    match band {
        "a" => Some("5 GHz"),
        "b" => Some("2.4 GHz"),
        _ => None,
    }
}
/// "French VF - AAC-LC stereo" → ("French VF", "AAC-LC stereo")
pub(crate) fn split_track(t: &str) -> (String, String) {
    match t.split_once(" - ") {
        Some((name, codec)) => (name.trim().to_string(), codec.split('(').next().unwrap_or_default().trim().to_string()),
        None => (t.to_string(), String::new()),
    }
}
pub(crate) fn duration_label(secs: i64) -> String {
    if secs >= 60 && secs % 60 == 0 {
        format!("{} min", secs / 60)
    } else {
        format!("{secs} s")
    }
}
/// 4328 → "1:12:08", 548.9 → "9:08": whole seconds gone, as Kodi counts them.
pub(crate) fn clock(secs: f64) -> String {
    let s = secs.max(0.0).floor() as i64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}
/// Kodi reports its audio stream as "English DTS-HD MA 5.1 - DTS-HD MA 5.1(side)": language and
/// stream name, then the codec with its channel layout. The codec part is what the badge shows.
pub(crate) fn audio_format(sound_mode: &str) -> String {
    let codec = sound_mode.rsplit(" - ").next().unwrap_or_default();
    codec.split('(').next().unwrap_or_default().trim().to_string()
}
/// The badge for Kodi's audio stream: a logo id and what the logo does not already say. The
/// stream name is free text from the file (often "DTS-HD MA 5.1", sometimes the driver's fallback
/// such as "Truehd_Atmos 8 channels"), so this matches families by keyword, most specific first.
pub(crate) fn codec_badge(stream: &str) -> (&'static str, String) {
    let s = format!(" {} ", stream.to_lowercase().replace(['_', '(', ')', ','], " "));
    let has = |words: &[&str]| words.iter().any(|w| s.contains(w));
    let ch = channels(&s);
    let with = |prefix: &str| [prefix, ch.as_str()].iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(" ");
    // Dolby and DTS show only their marks; the label names the format.
    if has(&["atmos"]) {
        let carrier = if has(&["truehd", "true hd"]) { "TrueHD Atmos" } else if has(&["dd+", "ddp", "eac3", "e-ac-3", "digital plus"]) { "Digital Plus Atmos" } else { "Atmos" };
        return ("dolby", with(carrier));
    }
    if has(&["dts:x", "dtsx", "dts-x", " ma x ", "dts x "]) {
        return ("dts", with(if has(&["imax"]) { "X IMAX" } else { "X" }));
    }
    if has(&["dts-hd ma", "dtshd ma", "dts-hd master", "dts hd ma", "dts ma ", "dts-ma"]) {
        return ("dts", with("HD MA"));
    }
    if has(&["dts-hd hra", "dtshd hra", "dts-hd high", "dts hd hra", "hra "]) {
        return ("dts", with("HD HRA"));
    }
    if has(&["dts-hd", "dtshd", "dts hd"]) {
        return ("dts", with("HD"));
    }
    if has(&[" dts", "dts ", " dca "]) {
        return ("dts", with(if has(&["dts-es", "dts es"]) { "ES" } else if has(&["96/24", "96 24"]) { "96/24" } else if has(&["express"]) { "Express" } else { "" }));
    }
    if has(&["truehd", "true hd", " mlp "]) {
        return ("dolby", with("TrueHD"));
    }
    if has(&["dd+", " ddp", "eac3", "e-ac-3", "digital plus"]) {
        return ("dolby", with("Digital Plus"));
    }
    if has(&[" ac3", " ac-3", "dolby digital", " dd "]) {
        return ("dolby", with("Digital"));
    }
    if has(&[" flac"]) {
        return ("flac", ch);
    }
    if has(&[" alac", "apple lossless"]) {
        return ("alac", ch);
    }
    if has(&[" opus"]) {
        return ("opus", ch);
    }
    if has(&[" mp3"]) {
        return ("mp3", ch);
    }
    if has(&["aac", "mp4a"]) {
        return ("aac", with(if has(&["he-aac", "he aac"]) { "HE" } else { "" }));
    }
    if has(&["pcm", " wav "]) {
        return ("", with("PCM"));
    }
    ("", stream.trim().to_string())
}
/// "5.1", "7.1", "Stereo"…, from either a layout or the driver's "N channels".
pub(crate) fn channels(s: &str) -> String {
    let words: Vec<&str> = s.split_whitespace().collect();
    if let Some(layout) = words.iter().find(|w| w.len() == 3 && w.as_bytes()[1] == b'.' && w.chars().all(|c| c.is_ascii_digit() || c == '.')) {
        return if *layout == "2.0" { "Stereo".into() } else { layout.to_string() };
    }
    if words.contains(&"stereo") {
        return "Stereo".into();
    }
    if words.contains(&"mono") {
        return "Mono".into();
    }
    match words.windows(2).find(|w| w[1].starts_with("channel")).and_then(|w| w[0].parse::<u32>().ok()) {
        Some(1) => "Mono".into(),
        Some(2) => "Stereo".into(),
        Some(n @ 3..=8) => format!("{}.1", n - 1),
        _ => String::new(),
    }
}
pub(crate) struct CodecLogo {
    pub(crate) path: SharedString,
    /// The part of the viewbox the mark fills: top, width, height.
    pub(crate) top: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
    /// How tall it is drawn, in logical pixels.
    pub(crate) size: f32,
    /// A wordmark with no descenders, set on the label's baseline instead of centred.
    pub(crate) baseline: bool,
}
pub(crate) fn codec_logo(app: &AppWindow, id: &str) -> CodecLogo {
    let c = app.global::<Codecs>();
    let lockup = |path: SharedString, width: f32| CodecLogo { path, top: 0.0, width, height: 24.0, size: 30.0, baseline: false };
    match id {
        // The simple-icons DTS mark sits in a 24x24 box with empty space above and below.
        "dolby" => CodecLogo { path: c.get_dolby_box(), top: 0.0, width: 33.03, height: 24.0, size: 20.0, baseline: false },
        "dts" => CodecLogo { path: app.global::<Logos>().get_dts(), top: 7.12, width: 24.0, height: 9.76, size: 17.0, baseline: true },
        "flac" => CodecLogo { size: 18.0, baseline: true, ..lockup(c.get_flac(), 49.63) },
        "opus" => lockup(c.get_opus(), 42.19),
        "alac" => lockup(c.get_alac(), 44.99),
        "mp3" => lockup(c.get_mp3(), 47.89),
        "aac" => lockup(c.get_aac(), 23.91),
        _ => lockup(SharedString::new(), 24.0),
    }
}
/// A brand mark found from an input's name, for inputs with no driver to give one.
pub(crate) fn input_logo(logos: &Logos, name: &str) -> SharedString {
    let n = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    if has(&["playstation", "ps4", "ps5"]) {
        logos.get_playstation()
    } else if has(&["xbox"]) {
        logos.get_xbox()
    } else if has(&["android", "shield", "google"]) {
        logos.get_android()
    } else if has(&["bluetooth"]) {
        logos.get_bluetooth()
    } else if has(&["heos", "music", "tuner", "phono", "cd"]) {
        logos.get_music()
    } else {
        logos.get_generic()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(clock(4328.0), "1:12:08");
        assert_eq!(clock(548.0), "9:08");
        assert_eq!(clock(548.9), "9:08");
        assert_eq!(split_track("French VF - AAC-LC stereo"), ("French VF".into(), "AAC-LC stereo".into()));
        let badge = |s: &str| { let (l, t) = codec_badge(s); (l, t) };
        assert_eq!(badge("DTS-HD MA 7.1"), ("dts", "HD MA 7.1".to_string()));
        assert_eq!(badge("TrueHD Atmos 7.1"), ("dolby", "TrueHD Atmos 7.1".to_string()));
        assert_eq!(badge("Truehd_Atmos 8 channels"), ("dolby", "TrueHD Atmos 7.1".to_string()));
        assert_eq!(badge("DD+ 5.1"), ("dolby", "Digital Plus 5.1".to_string()));
        assert_eq!(badge("Dolby Digital 5.1"), ("dolby", "Digital 5.1".to_string()));
        assert_eq!(badge("AAC-LC stereo"), ("aac", "Stereo".to_string()));
        assert_eq!(badge("DTS:X 7.1"), ("dts", "X 7.1".to_string()));
        assert_eq!(badge("Pcm_S24Le 2 channels"), ("", "PCM Stereo".to_string()));
        assert_eq!(badge("FLAC 2.0"), ("flac", "Stereo".to_string()));
        assert_eq!(audio_format("English DTS-HD MA 5.1 - DTS-HD MA 5.1(side)"), "DTS-HD MA 5.1");
        assert_eq!(codec_badge(&audio_format("English DTS-HD MA 5.1 - DTS-HD MA 5.1(side)")), ("dts", "HD MA 5.1".to_string()));
        assert_eq!(audio_format(""), "");
    }
}
