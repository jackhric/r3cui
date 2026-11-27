//! Kodi's ids, each its own type so a movie id can't be passed where an episode's is wanted.

use serde::{Deserialize, Serialize};

macro_rules! id {
    ($(#[$doc:meta])* $name:ident($inner:ty)) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub $inner);
    };
}

id!(/// `Player.Id`: 0 to 2.
    PlayerId(u8));
id!(MovieId(u32));
id!(EpisodeId(u32));
id!(TvShowId(u32));
id!(/// An audio stream's `index` in `Player.Audio.Stream`.
    AudioStreamIndex(u32));
id!(/// A subtitle's `index` in `Player.Subtitle`.
    SubtitleIndex(u32));
id!(/// A JSON-RPC request id, ours.
    RequestId(u32));

/// A library item that can be opened or refreshed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaId {
    Movie(MovieId),
    Episode(EpisodeId),
    TvShow(TvShowId),
}
