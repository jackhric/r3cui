//! The last list each driver gave for each kind of target, kept so its pickers open at once and still show what is
//! bound while the driver is offline. Written to `targets.json` beside `modules.json`.

use r3cui_sdk::api::{Target, TargetKind};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KnownTargets {
    lists: Vec<Known>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Known {
    driver: String,
    kind: TargetKind,
    targets: Vec<Target>,
}

impl KnownTargets {
    /// What was saved at `path`; nothing when there is no file or it doesn't parse (it is only a cache).
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    /// Written beside and renamed over, like `modules.json`.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let tmp = path.with_extension("json.tmp");
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&tmp, serde_json::to_string_pretty(self).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }

    pub fn get(&self, driver: &str, kind: TargetKind) -> Option<&[Target]> {
        self.lists.iter().find(|k| k.driver == driver && k.kind == kind).map(|k| k.targets.as_slice())
    }

    /// Keeps `targets` as the driver's list of `kind`; `false` when that is what was kept already.
    pub fn put(&mut self, driver: &str, kind: TargetKind, targets: Vec<Target>) -> bool {
        match self.lists.iter_mut().find(|k| k.driver == driver && k.kind == kind) {
            Some(k) if k.targets == targets => false,
            Some(k) => {
                k.targets = targets;
                true
            }
            None => {
                self.lists.push(Known { driver: driver.to_string(), kind, targets });
                true
            }
        }
    }

    /// The name the driver last gave the target `id`, in any of its lists.
    pub fn name(&self, driver: &str, id: &str) -> Option<&str> {
        self.lists.iter().filter(|k| k.driver == driver).flat_map(|k| &k.targets).find(|t| t.id == id).map(|t| t.name.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn amp() -> Vec<Target> {
        vec![Target { id: "media_player.amp".into(), name: "Amp".into() }]
    }

    #[test]
    fn lists_are_kept_per_driver_and_kind_and_survive_a_restart() {
        let dir = std::env::temp_dir().join(format!("known-targets-{}", std::process::id()));
        let path = dir.join("targets.json");
        let mut known = KnownTargets::default();
        assert!(known.put("homeassistant", TargetKind::MediaPlayer, amp()));
        assert!(!known.put("homeassistant", TargetKind::MediaPlayer, amp()), "unchanged");
        assert_eq!(known.get("homeassistant", TargetKind::Light), None);
        assert_eq!(known.name("homeassistant", "media_player.amp"), Some("Amp"));
        known.save(&path).unwrap();
        assert_eq!(KnownTargets::load(&path), known);
        assert_eq!(KnownTargets::load(&dir.join("missing.json")), KnownTargets::default());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
