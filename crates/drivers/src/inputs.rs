//! The input list as the remote shows it: the source's own list with the user's names, hiding, order and slots.

use crate::config::Modules;

/// The one input there is when nothing is set up as the source, so its slots still have a home.
pub const DEFAULT_INPUT: &str = "Default";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    pub id: String,
    pub name: String,
    pub hidden: bool,
    pub remote: Option<String>,
    pub ui: Option<String>,
    /// Renamed, hidden or placed by the user: not as the source reports it.
    pub changed: bool,
}

/// Every input, hidden ones included: the ones the user placed first, in their order, then the rest as the source
/// lists them. `reported` is `None` when there is no source.
pub fn inputs(reported: Option<&[String]>, m: &Modules) -> Vec<Input> {
    let Some(reported) = reported else { return vec![input(DEFAULT_INPUT, m)] };
    let placed = m.inputs.iter().filter(|e| e.placed && reported.contains(&e.id)).map(|e| e.id.as_str());
    let rest = reported.iter().map(String::as_str).filter(|id| m.input(id).is_none_or(|e| !e.placed));
    placed.chain(rest).map(|id| input(id, m)).collect()
}

fn input(id: &str, m: &Modules) -> Input {
    let entry = m.input(id);
    Input {
        id: id.to_string(),
        name: entry.and_then(|e| e.name.clone()).unwrap_or_else(|| id.to_string()),
        hidden: entry.is_some_and(|e| e.hidden),
        remote: entry.and_then(|e| e.remote.clone()),
        ui: entry.and_then(|e| e.ui.clone()),
        changed: entry.is_some_and(|e| e.name.is_some() || e.hidden || e.placed),
    }
}

/// The input that is on: the source's current one, or Default when there is no source.
pub fn current<'a>(all: &'a [Input], source: Option<&str>) -> Option<&'a Input> {
    match source {
        None => all.iter().find(|i| i.id == DEFAULT_INPUT),
        Some(s) => all.iter().find(|i| i.id == s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reported() -> Vec<String> {
        ["ANDROID TV", "KODI", "NET", "PLAYSTATION 5"].map(String::from).to_vec()
    }

    fn ids(list: &[Input]) -> Vec<&str> {
        list.iter().map(|i| i.id.as_str()).collect()
    }

    #[test]
    fn the_sources_order_with_the_users_changes() {
        let mut m = Modules::default();
        assert_eq!(ids(&inputs(Some(&reported()), &m)), ["ANDROID TV", "KODI", "NET", "PLAYSTATION 5"]);
        m.move_input(&reported(), "KODI", -1);
        m.rename("PLAYSTATION 5", "PS5");
        m.set_hidden("NET", true);
        let list = inputs(Some(&reported()), &m);
        assert_eq!(ids(&list), ["KODI", "ANDROID TV", "NET", "PLAYSTATION 5"]);
        assert!(list[0].changed && list[1].changed, "both were placed");
        assert!(list[2].hidden && list[2].changed);
        assert_eq!((list[3].name.as_str(), list[3].changed), ("PS5", true));
    }

    #[test]
    fn inputs_the_source_dropped_are_left_out_but_kept() {
        let mut m = Modules::default();
        m.set_slot("XBOX 360", crate::config::Slot::Ui, Some(crate::kodi::SPEC.id.into()));
        m.move_input(&["XBOX 360".into(), "KODI".into()], "KODI", -1);
        let list = inputs(Some(&reported()), &m);
        assert_eq!(ids(&list), ["KODI", "ANDROID TV", "NET", "PLAYSTATION 5"]);
        assert!(m.input("XBOX 360").is_some());
    }

    #[test]
    fn no_source_means_one_default_input() {
        let mut m = Modules::default();
        m.set_slot(DEFAULT_INPUT, crate::config::Slot::Ui, Some(crate::kodi::SPEC.id.into()));
        let list = inputs(None, &m);
        assert_eq!(ids(&list), [DEFAULT_INPUT]);
        assert_eq!(current(&list, None).unwrap().ui.as_deref(), Some(crate::kodi::SPEC.id));
        assert_eq!(current(&inputs(Some(&reported()), &m), Some("KODI")).unwrap().ui, None);
    }
}
