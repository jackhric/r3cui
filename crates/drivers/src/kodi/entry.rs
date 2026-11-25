//! One title on the wall: Kodi's list item turned into what the details under the wall show.

use crate::api::LibraryItem;
use kodi::methods::library::FileItem;

/// `poster` is the artwork's full URL.
pub(crate) fn entry(it: &FileItem, shows: bool, poster: String) -> LibraryItem {
    let year = it.year;
    let mpaa = certificate(&it.mpaa);
    let (meta, detail, progress) = if shows {
        let (eps, seen, seasons) = (it.episode, it.watchedepisodes, it.season);
        let meta = format!("{year} · {seasons} season{}", if seasons == 1 { "" } else { "s" });
        (meta, format!("{seen} of {eps} episodes watched"), if eps > 0 { seen as f32 / eps as f32 } else { 0.0 })
    } else {
        let mins = it.runtime / 60;
        let meta = format!("{year} · {}h {:02}m", mins / 60, mins % 60);
        let (pos, total) = (it.resume.position, it.resume.total);
        let detail = if pos > 0.0 {
            format!("Resume from {}:{:02}", pos as i64 / 3600, pos as i64 / 60 % 60)
        } else if it.playcount > 0 {
            "Watched".into()
        } else {
            String::new()
        };
        (meta, detail, if total > 0.0 { (pos / total) as f32 } else { 0.0 })
    };
    LibraryItem {
        title: if it.title.is_empty() { it.label.clone() } else { it.title.clone() },
        meta,
        rating: if it.rating > 0.0 { format!("{:.1}", it.rating) } else { String::new() },
        tail: mpaa,
        genre: it.genre.iter().take(3).cloned().collect::<Vec<_>>().join(" · "),
        plot: it.plot.clone(),
        detail,
        progress,
        poster,
    }
}

/// Kodi's certification ("Rated PG-13", "Not Rated", "TV-MA") as the code the badges are named by.
fn certificate(raw: &str) -> String {
    let code = raw.trim().trim_start_matches("Rated ").trim().to_uppercase();
    match code.as_str() {
        "NOT RATED" | "UNRATED" | "NR" => "NR".into(),
        _ => code,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn certificates_match_the_badge_names() {
        assert_eq!(certificate("Rated PG-13"), "PG-13");
        assert_eq!(certificate("tv-ma"), "TV-MA");
        assert_eq!(certificate("Not Rated"), "NR");
        assert_eq!(certificate(""), "");
    }
}
