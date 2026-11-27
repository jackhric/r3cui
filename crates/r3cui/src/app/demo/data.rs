//! Everything the demo shows, made up, in one place: the driver plays it through the real frontend, and the
//! preview stills read the same values.

use r3cui_sdk::api::*;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const ANDROID_TV: &str = "ANDROID TV";
/// The one input shown with a driver's mark, as a real Kodi would; the others get their brand's.
pub const KODI: &str = "KODI";
pub const PLAYSTATION: &str = "PLAYSTATION 5";
pub const XBOX: &str = "XBOX 360";
pub const INPUTS: [&str; 4] = [ANDROID_TV, KODI, PLAYSTATION, XBOX];
pub const START_INPUT: &str = PLAYSTATION;
pub const RECEIVER: &str = "demo.receiver";
pub const PIPELINE: (&str, &str) = ("demo.assist", "Demo Assist");
/// What the made-up assistant hears and answers, a moment after the user lets go.
pub const VOICE_HEARD: &str = "Turn on the theater lights";
pub const VOICE_ANSWER: &str = "Turned on the theater lights";
pub const VOICE_THINKING: Duration = Duration::from_millis(900);

pub const START_LEVEL: i32 = 62;
pub const LEVEL_LIMIT: i32 = 72;
/// How long the receiver takes to switch input, so the screen's "switching" shows.
pub const SWITCH: Duration = Duration::from_millis(1500);

pub const FILM_TITLE: &str = "Mad Max: Fury Road";
pub const FILM_LENGTH: Duration = Duration::from_secs(7200);
/// Where in the film playback starts.
pub const RESUME_AT: Duration = Duration::from_secs(2788);
/// The audio streams as (label, detail); the first plays.
pub const AUDIO: [(&str, &str); 3] = [("English", "DTS-HD MA 7.1"), ("English Commentary", "AAC-LC stereo"), ("French", "Dolby Digital 5.1")];
/// As many as a typical remux carries, so the picker has to scroll; the first is the one showing.
pub const SUBTITLES: [&str; 19] = [
    "English", "English SDH", "French · Forced", "French", "German", "Spanish", "Spanish (Latin America)", "Italian", "Portuguese (Brazil)", "Dutch",
    "Danish", "Finnish", "Norwegian", "Swedish", "Polish", "Czech", "Japanese", "Korean", "Chinese (Simplified)",
];
pub const SUBTITLE_SHOWING: usize = 0;

pub const SHELF: [(&str, &str); 6] = [
    ("2001: A Space Odyssey", "1968 · 2h 29m"),
    ("Alien", "1979 · 1h 57m"),
    ("Arrival", "2016 · 1h 56m"),
    ("Blade Runner", "1982 · 1h 57m"),
    ("Mad Max: Fury Road", "2015 · 2h 0m"),
    ("The Godfather", "1972 · 2h 55m"),
];
/// The shelf is laid out this many to a row.
pub const COLUMNS: usize = 3;

/// The links the stills' warning is worked out from, where no driver reports them.
pub const LINKS: [&str; 2] = ["Home Assistant", "Kodi"];

pub const BATTERY: i32 = 81;
pub const WIFI_HOME: &str = "HomeNetwork";
/// The networks in range, as (name, bars out of 4).
pub const WIFI: [(&str, i32); 3] = [(WIFI_HOME, 3), ("Guest", 4), ("SmartHome", 2)];
/// The preferred band as the core names it: "b" is 2.4 GHz.
pub const WIFI_BAND: &str = "b";

/// The remote's software, as the made-up core and GitHub report it: a firmware update and an r3cui update, both
/// waiting.
pub const REMOTE_ADDRESS: &str = "192.168.1.40";
pub const FIRMWARE_INSTALLED: &str = "2.8.4";
pub const FIRMWARE_UPDATE_ID: &str = "demo-firmware-2.9.0";
pub const FIRMWARE_UPDATE: &str = "2.9.0";
pub const FIRMWARE_RELEASE_DATE: &str = "2026-11-10";
pub const FIRMWARE_SIZE: u64 = 118_000_000;
pub const FIRMWARE_NOTES: &str = "Wi-Fi reconnects faster after a router restart, the charging light no longer flickers at 99%, and the dock stays paired through firmware updates.";
pub const FIRMWARE_STEPS: u32 = 4;
/// The made-up r3cui release: the demo runs as the build it is, and offers the next minor version as this.
pub const R3_RELEASE_DATE: &str = "2026-11-20";
pub const R3_SIZE: u64 = 6_800_000;
/// The PINs the made-up core hands out in turn, so a recording shows the same ones every time.
pub const WEB_PINS: [&str; 3] = ["4827", "1593", "7306"];
/// How long each made-up install phase takes: the download, the installation, then the restart that never comes.
pub const DOWNLOAD_TIME: Duration = Duration::from_millis(4000);
pub const INSTALL_TIME: Duration = Duration::from_millis(5000);
pub const RESTART_TIME: Duration = Duration::from_millis(2500);

pub fn film(now: Instant) -> Media {
    let track = |label: &str, detail: &str| Track { label: label.into(), detail: detail.into() };
    Media {
        title: FILM_TITLE.into(),
        subtitle: String::new(),
        paused: false,
        position: RESUME_AT,
        measured_at: now,
        total: FILM_LENGTH,
        format: AUDIO[0].1.into(),
        poster: String::new(),
        backdrop: String::new(),
        audio: AUDIO.iter().map(|(l, d)| track(l, d)).collect(),
        audio_current: Some(0),
        subtitles: SUBTITLES.iter().map(|l| track(l, "")).collect(),
        subtitle_current: Some(SUBTITLE_SHOWING),
    }
}

pub fn shelf() -> Arc<LibraryList> {
    let items = SHELF
        .iter()
        .map(|(title, meta)| LibraryItem {
            title: (*title).into(),
            meta: (*meta).into(),
            rating: "8.1".into(),
            tail: "R".into(),
            genre: "Science Fiction".into(),
            plot: String::new(),
            detail: String::new(),
            progress: 0.0,
            poster: String::new(),
        })
        .collect();
    Arc::new(LibraryList { id: u64::MAX, heading: MOVIES_HEADING.into(), count: format!("{} films", SHELF.len()), items })
}
