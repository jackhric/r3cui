//! Keeping the film's title steady against the Kodi driver's temporary titles.

use std::time::{Duration, Instant};

/// The Kodi driver swaps the title for the audio/subtitle track names or the chapter name whenever one changes
/// (its "show track name" setting), and restarts that flash with every update, so it can outlast any wait. So the
/// title comes from Kodi itself (`Player.GetItem`), asked each time the driver's changes; once Kodi has answered
/// for a film the driver's titles are ignored until the film changes. Until then, or with Kodi out of reach, the
/// driver's flashes are recognised and a different title has to last before it is believed.
#[derive(Default)]
pub(crate) struct StableTitle {
    media: String,
    pub(crate) shown: String,
    pending: Option<(String, Instant)>,
    /// The driver's title as last reported, and the media and title Kodi was last asked about.
    raw: String,
    asked: String,
    /// Kodi has named the media playing.
    from_kodi: bool,
}
impl StableTitle {
    const SETTLE: Duration = Duration::from_secs(7);

    /// The media to ask Kodi the title of, when the driver's title has changed since Kodi was last asked.
    pub(crate) fn lookup(&mut self) -> Option<String> {
        let key = format!("{}|{}", self.media, self.raw);
        if self.raw.is_empty() || key == self.asked {
            return None;
        }
        self.asked = key;
        Some(self.media.clone())
    }

    /// Kodi's own title for `media`.
    pub(crate) fn answer(&mut self, media: &str, title: String) {
        if media == self.media {
            self.shown = title;
            self.pending = None;
            self.from_kodi = true;
        }
    }

    /// `tracks` is what the driver reports as the artist, `streams` the audio tracks' names and `chapter` the
    /// chapter playing: the texts it flashes in place of the title.
    pub(crate) fn feed(&mut self, media: &str, title: &str, tracks: &str, streams: &[String], chapter: &str, now: Instant) {
        self.raw = title.to_string();
        if media != self.media {
            self.from_kodi = false;
        } else if self.from_kodi {
            return;
        }
        let is_tracks = (!tracks.is_empty() && title == tracks) || (!chapter.is_empty() && title == chapter) || names_stream(title, streams);
        if media != self.media {
            self.media = media.to_string();
            self.shown = if is_tracks { String::new() } else { title.to_string() };
            self.pending = None;
        } else if is_tracks || title == self.shown {
            self.pending = None;
        } else if self.shown.is_empty() {
            self.shown = title.to_string();
        } else {
            match &self.pending {
                Some((t, since)) if t == title && now.duration_since(*since) >= Self::SETTLE => {
                    self.shown = title.to_string();
                    self.pending = None;
                }
                Some((t, _)) if t == title => {}
                _ => self.pending = Some((title.to_string(), now)),
            }
        }
    }
}
/// Whether `title` is the driver's flash of the audio track's name: the name capitalised word by word, perhaps
/// with the subtitle's after it. The track list carries the same name behind its language.
pub(crate) fn names_stream(title: &str, streams: &[String]) -> bool {
    let title = title.to_lowercase();
    streams.iter().any(|s| {
        let s = s.to_lowercase();
        let name = s.split_once(' ').map_or("", |(_, name)| name);
        (!name.is_empty() && title.starts_with(name)) || title.starts_with(&s)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporary_titles_are_ignored() {
        let t0 = Instant::now();
        let mut t = StableTitle::default();
        // Playback starts: the driver flashes the track names first.
        t.feed("m1", "English - English", "English - English", &[], "", t0);
        assert_eq!(t.shown, "");
        t.feed("m1", "Patton", "English - English", &[], "", t0);
        assert_eq!(t.shown, "Patton");
        // A chapter name appears for a few seconds, then the title returns.
        t.feed("m1", "Chapter 4", "English - English", &[], "", t0);
        t.feed("m1", "Chapter 4", "English - English", &[], "", t0 + Duration::from_secs(4));
        assert_eq!(t.shown, "Patton");
        t.feed("m1", "Patton", "English - English", &[], "", t0 + Duration::from_secs(5));
        // A change that lasts is real.
        t.feed("m1", "Patton (Extended)", "", &[], "", t0 + Duration::from_secs(10));
        t.feed("m1", "Patton (Extended)", "", &[], "", t0 + Duration::from_secs(18));
        assert_eq!(t.shown, "Patton (Extended)");
        // Another film takes over at once.
        t.feed("m2", "Alien", "", &[], "", t0 + Duration::from_secs(20));
        assert_eq!(t.shown, "Alien");
    }

    #[test]
    fn kodis_title_wins_over_the_drivers() {
        let t0 = Instant::now();
        let streams = ["English Restored and Re-mixed / DTS-HD Master Audio - DTS-HD MA 5.1(side)".to_string()];
        let flash = "Restored And Re-Mixed / Dts-Hd Master Audio - Dts-Hd Ma 5.1(Side)";
        let mut t = StableTitle::default();
        // A film starting on a flash: Kodi is asked once, and its answer is shown.
        t.feed("m1", flash, "English", &streams, "", t0);
        assert_eq!(t.shown, "");
        assert_eq!(t.lookup().as_deref(), Some("m1"));
        assert_eq!(t.lookup(), None);
        t.answer("m1", "2001: A Space Odyssey".into());
        assert_eq!(t.shown, "2001: A Space Odyssey");
        // Whatever the driver reports for the same film afterwards, however long, changes nothing; each change
        // asks Kodi again.
        for s in [1, 9, 20] {
            t.feed("m1", "Something Else Entirely", "English", &streams, "", t0 + Duration::from_secs(s));
        }
        assert_eq!(t.shown, "2001: A Space Odyssey");
        assert_eq!(t.lookup().as_deref(), Some("m1"));
        // An answer for a film that has since given way is dropped.
        t.feed("m2", "Alien", "", &[], "", t0);
        t.answer("m1", "2001: A Space Odyssey".into());
        assert_eq!(t.shown, "Alien");
    }

    #[test]
    fn stream_and_chapter_flashes_never_become_the_title() {
        let t0 = Instant::now();
        let streams = ["English Restored and Re-mixed / DTS-HD Master Audio / 5.1 - DTS-HD MA 5.1(side)".to_string()];
        let mut t = StableTitle::default();
        t.feed("m1", "2001: A Space Odyssey", "English - English", &streams, "", t0);
        // The driver keeps the flash up as long as updates come, longer than the settling time.
        let flash = "Restored And Re-Mixed / Dts-Hd Master Audio / 5.1 - Dts-Hd Ma 5.1(Side) - Sdh";
        for s in [1, 5, 9, 14] {
            t.feed("m1", flash, "English - English", &streams, "", t0 + Duration::from_secs(s));
        }
        assert_eq!(t.shown, "2001: A Space Odyssey");
        // A new film starting on a flash waits for its real title.
        t.feed("m2", flash, "English - English", &streams, "", t0 + Duration::from_secs(20));
        assert_eq!(t.shown, "");
        t.feed("m2", "Moon", "English - English", &streams, "", t0 + Duration::from_secs(21));
        assert_eq!(t.shown, "Moon");
        for s in [22, 30] {
            t.feed("m2", "The Base", "English - English", &streams, "The Base", t0 + Duration::from_secs(s));
        }
        assert_eq!(t.shown, "Moon");
    }
}
