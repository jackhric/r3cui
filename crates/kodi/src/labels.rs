//! Words for the screen: language names for Kodi's ISO 639 codes, and labels for audio and subtitle tracks.

use crate::wire::{AudioStream, SubtitleStream};

/// Kodi reports subtitle languages as ISO 639-2 codes ("eng", "ger" or "deu"); the picker shows the language's name.
pub fn language_name(code: &str) -> Option<&'static str> {
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


/// "English (SDH)", "French · Forced": the language, the track's own name where it adds something, and whether it's
/// forced.
pub fn subtitle_label(s: &SubtitleStream) -> String {
    let mut label = named(&s.language, &s.name);
    if s.forced {
        label.push_str(" · Forced");
    }
    label
}

/// "English (Commentary by …)", or the language alone.
pub fn audio_label(a: &AudioStream) -> String {
    named(&a.language, &a.name)
}

/// What the codec badge reads: Kodi's codec id and channel count, such as "dtshd_ma 6 channels".
pub fn audio_format(a: &AudioStream) -> String {
    if a.channels > 0 { format!("{} {} channels", a.codec, a.channels) } else { a.codec.clone() }
}

fn named(code: &str, name: &str) -> String {
    let (name, code) = (name.trim(), code.trim());
    let language = language_name(code).map_or_else(|| code.to_string(), str::to_string);
    match (name.is_empty(), language.is_empty()) {
        (false, false) if !name.to_lowercase().contains(&language.to_lowercase()) => format!("{language} ({name})"),
        (false, _) => name.to_string(),
        (true, false) => language,
        (true, true) => "Unnamed".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::SubtitleIndex;

    fn sub(name: &str, language: &str, forced: bool) -> SubtitleStream {
        SubtitleStream { index: SubtitleIndex(0), name: name.into(), language: language.into(), forced, impaired: false, default: false }
    }

    #[test]
    fn labels() {
        assert_eq!(subtitle_label(&sub("SDH", "eng", false)), "English (SDH)");
        assert_eq!(subtitle_label(&sub("English", "eng", false)), "English");
        assert_eq!(subtitle_label(&sub("", "fre", true)), "French · Forced");
        assert_eq!(subtitle_label(&sub("", "", false)), "Unnamed");
    }
}
