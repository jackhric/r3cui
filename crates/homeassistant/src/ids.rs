use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntityId(pub String);

impl EntityId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// "media_player" for `media_player.theater_receiver`.
    pub fn domain(&self) -> &str {
        self.0.split_once('.').map_or(&self.0, |(domain, _)| domain)
    }
}

impl From<&str> for EntityId {
    fn from(id: &str) -> Self {
        Self(id.to_owned())
    }
}

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_is_before_the_dot() {
        assert_eq!(EntityId::from("media_player.theater_receiver").domain(), "media_player");
        assert_eq!(EntityId::from("odd").domain(), "odd");
        assert_eq!(serde_json::to_string(&EntityId::from("light.desk")).unwrap(), "\"light.desk\"");
    }
}
