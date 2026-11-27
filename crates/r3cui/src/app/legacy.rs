//! What `settings.json` held before drivers were set up in `modules.json`, and the first `modules.json` made from it.
//! The one place that knows Kodi's old settings, so it lives with the app, which lists Kodi anyway.

use crate::host::{Modules, Slot, DEFAULT_INPUT};
use kodi::driver::{field, SPEC as KODI};

/// The old hardwired input Kodi was tied to, before inputs were configurable.
const KODI_INPUT: &str = "KODI";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Legacy {
    pub kodi_host: Option<String>,
    pub kodi_http_port: Option<u16>,
    pub kodi_ws_port: Option<u16>,
    pub hidden_inputs: Vec<String>,
}

impl Legacy {
    /// Kodi in the remote and UI slots of its input (and of Default, for a setup with no source yet), and the hidden
    /// inputs. `None` when the old settings held neither.
    pub(crate) fn modules(&self) -> Option<Modules> {
        if self.kodi_host.is_none() && self.hidden_inputs.is_empty() {
            return None;
        }
        let mut m = Modules::default();
        if let Some(host) = &self.kodi_host {
            let id = m.add_driver(KODI.id, Default::default());
            let port = |p: Option<u16>| p.map(|p| p.to_string()).unwrap_or_default();
            for (key, value) in [(field::HOST, host.clone()), (field::HTTP_PORT, port(self.kodi_http_port)), (field::WS_PORT, port(self.kodi_ws_port))] {
                if let Some(f) = KODI.field(key) {
                    m.set_field(&id, f, &value);
                }
            }
            for input in [KODI_INPUT, DEFAULT_INPUT] {
                m.set_slot(input, Slot::Remote, Some(id.clone()));
                m.set_slot(input, Slot::Ui, Some(id.clone()));
            }
        }
        for h in &self.hidden_inputs {
            m.set_hidden(h, true);
        }
        Some(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn old_settings_fill_a_first_modules_json() {
        let old = Legacy { kodi_host: Some("theater".into()), kodi_ws_port: Some(9091), hidden_inputs: vec!["NET".into()], ..Default::default() };
        let m = old.modules().unwrap();
        let kodi = &m.drivers[KODI.id];
        assert_eq!(kodi.text(field::HOST).as_deref(), Some("theater"));
        assert_eq!(kodi.fields[field::WS_PORT], Value::from(9091), "ports are numbers");
        assert_eq!(kodi.text(field::HTTP_PORT), None);
        assert_eq!(m.input(KODI_INPUT).unwrap().ui.as_deref(), Some(KODI.id));
        assert_eq!(m.input(DEFAULT_INPUT).unwrap().remote.as_deref(), Some(KODI.id));
        assert!(m.input("NET").unwrap().hidden);
        assert_eq!(Legacy::default().modules(), None);
    }
}
