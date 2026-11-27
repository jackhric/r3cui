#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Link {
    #[default]
    Connecting,
    Up,
    /// The TV turned our certificate away: run `pair` again.
    NotPaired,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Volume {
    pub level: u32,
    pub max: u32,
    pub muted: bool,
}

/// What the TV last told us. Everything but `link` is reset when the connection drops, so a stale "on" never lingers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TvState {
    pub link: Link,
    pub on: bool,
    pub current_app: Option<String>,
    pub volume: Option<Volume>,
    pub model: Option<String>,
}

impl TvState {
    pub(crate) fn down(link: Link) -> Self {
        Self { link, ..Self::default() }
    }
}
