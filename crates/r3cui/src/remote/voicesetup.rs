//! The r3cui bridge on the remote, set up in one go: install the one that r3cui's archive carries (or bring
//! it up to this version), set it up so the core has an instance of it, add its voice assistant entity, then turn the
//! microphone on and make that entity the remote's voice assistant. Every step is safe to repeat.

use crate::app::software::Access;
use crate::remote::rest::{Rest, RestError};
use crate::remote::software::{grant, revoke, rest};
use crate::coreapi::{Core, VoiceAssistantUpdate, VoiceControlUpdate};
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use r3cui_bridge::link::{BRIDGE_DRIVER_ID, VOICE_ENTITY_ID};

/// Next to the binary in r3cui's archive; `packaging/pack.sh` puts it there.
const ARCHIVE: &str = "r3cui-bridge.tar.gz";
/// The bridge is built from the same workspace, so it carries r3cui's version.
const VERSION: &str = env!("CARGO_PKG_VERSION");
const DRIVERS_PATH: &str = "/api/intg/drivers";
/// What the bridge was called before it took on more than voice; a remote set up under it has it replaced.
const LEGACY_DRIVER_ID: &str = "r3cui_voice";
const INSTALL_PATH: &str = "/api/intg/install";
const UPDATE: (&str, &str) = ("update", "true");
const SETUP_PATH: &str = "/api/intg/setup";
const INSTANCES_PATH: &str = "/api/intg/instances";
/// The bridge has nothing to ask, so setup ends at once, and its entity shows a moment later; this is ample for both.
const SETUP_WITHIN: Duration = Duration::from_secs(30);
const SETUP_POLL: Duration = Duration::from_millis(500);
/// Every entity the integration offers, configured or not, asked of the integration afresh.
const OFFERED: &str = "entities?filter=ALL&reload=true";

/// What the System page's voice tile shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum State {
    Ready,
    /// The microphone is off, or the voice assistant is someone else's or none.
    Absent,
}

pub(crate) fn entity_of(integration_id: &str) -> String {
    format!("{integration_id}.{VOICE_ENTITY_ID}")
}

fn is_bridge(entity_id: &str) -> bool {
    entity_id.split('.').next() == Some(BRIDGE_DRIVER_ID)
}

pub(crate) async fn state(core: &Core) -> Result<State, String> {
    let cfg = core.get_voice_control_cfg().await.map_err(|e| e.to_string())?;
    let bridge = cfg.voice_assistant.active.is_some_and(|a| is_bridge(a.entity_id.as_str()));
    Ok(if cfg.microphone && bridge { State::Ready } else { State::Absent })
}

pub(crate) async fn install(core: &Core, access: Access) -> Result<(), String> {
    let archive = archive()?;
    grant(core, &access).await?;
    let rest = rest(&access);
    let set_up = tokio::task::spawn_blocking(move || set_up(&rest, &archive)).await.map_err(|e| e.to_string());
    revoke(core, &access).await;
    let entity = set_up?.map_err(|e| e.to_string())?;
    let update = VoiceControlUpdate {
        microphone: Some(true),
        voice_assistant: Some(VoiceAssistantUpdate { entity_id: entity.clone(), profile_id: None, speech_response: None }),
    };
    let cfg = core.set_voice_control_cfg(&update).await.map_err(|e| format!("Could not turn voice on: {e}"))?;
    // An entity the core does not know clears the voice assistant rather than failing.
    match cfg.voice_assistant.active {
        Some(a) if cfg.microphone && a.entity_id.as_str() == entity => Ok(()),
        _ => Err(format!("The remote did not take {entity} as its voice assistant")),
    }
}

fn archive() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let path = exe.with_file_name(ARCHIVE);
    if path.is_file() { Ok(path) } else { Err("This build of r3cui does not carry the r3cui bridge".into()) }
}

#[derive(Deserialize)]
struct Driver {
    #[serde(default)]
    version: String,
}

#[derive(Deserialize)]
struct Instance {
    integration_id: String,
    driver_id: String,
}

#[derive(Deserialize)]
struct Offered {
    entity_id: String,
}

#[derive(Deserialize)]
struct Setup {
    state: String,
    #[serde(default)]
    error: Option<String>,
}

/// The REST half, blocking: returns the voice assistant's entity id.
fn set_up(rest: &Rest, archive: &std::path::Path) -> Result<String, RestError> {
    remove_legacy(rest)?;
    match rest.get::<Driver>(&format!("{DRIVERS_PATH}/{BRIDGE_DRIVER_ID}"))? {
        None => drop(rest.upload(INSTALL_PATH, None, archive)?),
        Some(d) if d.version != VERSION => drop(rest.upload(INSTALL_PATH, Some(UPDATE), archive)?),
        Some(_) => {}
    }
    let integration = match instance(rest)? {
        Some(id) => id,
        None => {
            run_setup(rest)?;
            instance(rest)?.ok_or_else(|| RestError::Unexpected("set up, but no integration to show for it".into()))?
        }
    };
    let entities = format!("{INSTANCES_PATH}/{integration}/entities");
    // Right after setup the core may not have the bridge's entities yet, and it skips an id it does not know without
    // saying so.
    wait_for_entity(rest, &format!("{INSTANCES_PATH}/{integration}/{OFFERED}"))?;
    let _: serde_json::Value = rest.post(&entities, &json!([VOICE_ENTITY_ID]))?;
    Ok(entity_of(&integration))
}

fn remove_legacy(rest: &Rest) -> Result<(), RestError> {
    let driver = format!("{DRIVERS_PATH}/{LEGACY_DRIVER_ID}");
    if rest.get::<Driver>(&driver)?.is_none() {
        return Ok(());
    }
    let all: Vec<Instance> = rest.get(INSTANCES_PATH)?.unwrap_or_default();
    for old in all.iter().filter(|i| i.driver_id == LEGACY_DRIVER_ID) {
        rest.remove(&format!("{INSTANCES_PATH}/{}", old.integration_id))?;
    }
    rest.remove(&driver)
}

fn wait_for_entity(rest: &Rest, offered: &str) -> Result<(), RestError> {
    let until = Instant::now() + SETUP_WITHIN;
    loop {
        let all: Vec<Offered> = rest.get(offered)?.unwrap_or_default();
        if all.iter().any(|e| e.entity_id == VOICE_ENTITY_ID) {
            return Ok(());
        }
        if Instant::now() >= until {
            return Err(RestError::Unexpected(format!("the integration did not offer its {VOICE_ENTITY_ID} entity")));
        }
        std::thread::sleep(SETUP_POLL);
    }
}

fn instance(rest: &Rest) -> Result<Option<String>, RestError> {
    let all: Vec<Instance> = rest.get(INSTANCES_PATH)?.unwrap_or_default();
    Ok(all.into_iter().find(|i| i.driver_id == BRIDGE_DRIVER_ID).map(|i| i.integration_id))
}

fn run_setup(rest: &Rest) -> Result<(), RestError> {
    let mut setup: Setup = rest.post(SETUP_PATH, &json!({ "driver_id": BRIDGE_DRIVER_ID, "reconfigure": false }))?;
    let path = format!("{SETUP_PATH}/{BRIDGE_DRIVER_ID}");
    let until = Instant::now() + SETUP_WITHIN;
    loop {
        match setup.state.as_str() {
            "OK" => return Ok(()),
            "ERROR" => return Err(RestError::Unexpected(format!("setup failed: {}", setup.error.unwrap_or_default()))),
            "WAIT_USER_ACTION" => setup = rest.put(&path, &json!({ "confirm": true }))?,
            _ if Instant::now() >= until => return Err(RestError::Unexpected(format!("setup still {} after {SETUP_WITHIN:?}", setup.state))),
            _ => {
                std::thread::sleep(SETUP_POLL);
                // Once it is done, the core may forget the setup altogether.
                match rest.get::<Setup>(&path)? {
                    Some(s) => setup = s,
                    None => return Ok(()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::selfupdate::fake_http::{Canned, FakeHttp};

    fn tarball() -> PathBuf {
        let path = std::env::temp_dir().join(format!("r3cui-voicesetup-{}.tar.gz", std::process::id()));
        std::fs::write(&path, b"BRIDGE").unwrap();
        path
    }

    fn calls(http: &FakeHttp) -> Vec<String> {
        http.requests().iter().map(|r| format!("{} {}", r.method, r.target)).collect()
    }

    const DRIVER: &str = "/api/intg/drivers/r3cui_bridge";
    const LEGACY: &str = "/api/intg/drivers/r3cui_voice";
    const LEGACY_INSTANCE: &str = "/api/intg/instances/r3cui_voice.main";
    const ENTITIES: &str = "/api/intg/instances/r3cui_bridge.main/entities";
    const OFFER: &str = r#"[{"entity_id":"voice","entity_type":"voice_assistant"}]"#;
    const ONE: &str = r#"[{"integration_id":"hass.main","driver_id":"hass"},{"integration_id":"r3cui_bridge.main","driver_id":"r3cui_bridge"}]"#;

    #[test]
    fn a_fresh_remote_gets_the_bridge_installed_set_up_and_its_entity_added() {
        let http = FakeHttp::start(vec![
            Canned::answer("POST", INSTALL_PATH, 201, r#"{"driver_id":"r3cui_bridge"}"#),
            Canned::answer("POST", SETUP_PATH, 201, r#"{"id":"r3cui_bridge","state":"OK"}"#),
            Canned::ok(INSTANCES_PATH, ONE),
            Canned::ok(ENTITIES, OFFER),
            Canned::answer("POST", ENTITIES, 201, r#"["r3cui_bridge.main.voice"]"#),
        ]);
        let rest = Rest::at(vec![format!("http://{}", http.addr())], "1");
        assert_eq!(set_up(&rest, &tarball()).unwrap(), "r3cui_bridge.main.voice");
        let offered = format!("GET {ENTITIES}?filter=ALL&reload=true");
        assert_eq!(calls(&http), [format!("GET {LEGACY}"), format!("GET {DRIVER}"), format!("POST {INSTALL_PATH}"), format!("GET {INSTANCES_PATH}"), offered, format!("POST {ENTITIES}")]);
        assert_eq!(http.requests()[5].body, br#"["voice"]"#);
    }

    #[test]
    fn an_older_bridge_is_updated_in_place_and_a_current_one_left_alone() {
        let routes = |version: &str| {
            vec![
                Canned::ok(DRIVER, format!(r#"{{"driver_id":"r3cui_bridge","version":"{version}"}}"#)),
                Canned::answer("POST", INSTALL_PATH, 201, "{}"),
                Canned::ok(INSTANCES_PATH, ONE),
                Canned::ok(ENTITIES, OFFER),
                Canned::answer("POST", ENTITIES, 201, "[]"),
            ]
        };
        let old = FakeHttp::start(routes("0.0.1"));
        set_up(&Rest::at(vec![format!("http://{}", old.addr())], "1"), &tarball()).unwrap();
        assert_eq!(calls(&old)[2], format!("POST {INSTALL_PATH}?update=true"));
        let current = FakeHttp::start(routes(VERSION));
        set_up(&Rest::at(vec![format!("http://{}", current.addr())], "1"), &tarball()).unwrap();
        assert!(!calls(&current).iter().any(|c| c.contains(INSTALL_PATH)), "{:?}", calls(&current));
    }

    #[test]
    fn the_legacy_voice_driver_is_removed_with_its_instances_before_the_bridge_goes_in() {
        let http = FakeHttp::start(vec![
            Canned::ok(LEGACY, r#"{"driver_id":"r3cui_voice","version":"0.1.0"}"#),
            Canned::ok(INSTANCES_PATH, r#"[{"integration_id":"hass.main","driver_id":"hass"},{"integration_id":"r3cui_voice.main","driver_id":"r3cui_voice"},{"integration_id":"r3cui_bridge.main","driver_id":"r3cui_bridge"}]"#),
            Canned::answer("DELETE", LEGACY_INSTANCE, 204, ""),
            Canned::answer("DELETE", LEGACY, 204, ""),
            Canned::answer("POST", INSTALL_PATH, 201, "{}"),
            Canned::ok(ENTITIES, OFFER),
            Canned::answer("POST", ENTITIES, 201, "[]"),
        ]);
        let rest = Rest::at(vec![format!("http://{}", http.addr())], "1");
        set_up(&rest, &tarball()).unwrap();
        let seen = calls(&http);
        assert_eq!(seen[..5], [format!("GET {LEGACY}"), format!("GET {INSTANCES_PATH}"), format!("DELETE {LEGACY_INSTANCE}"), format!("DELETE {LEGACY}"), format!("GET {DRIVER}")]);
        assert!(seen.contains(&format!("POST {INSTALL_PATH}")), "{seen:?}");
    }

    #[test]
    fn a_failed_setup_says_why() {
        let http = FakeHttp::start(vec![
            Canned::ok(DRIVER, format!(r#"{{"version":"{VERSION}"}}"#)),
            Canned::ok(INSTANCES_PATH, "[]"),
            Canned::answer("POST", SETUP_PATH, 201, r#"{"state":"ERROR","error":"OTHER"}"#),
        ]);
        let err = set_up(&Rest::at(vec![format!("http://{}", http.addr())], "1"), &tarball()).unwrap_err();
        assert!(err.to_string().contains("OTHER"), "{err}");
    }

    #[test]
    fn only_the_bridges_entity_counts_as_r3cuis_voice() {
        assert!(is_bridge(&entity_of("r3cui_bridge.main")));
        assert!(!is_bridge("hass.main.assist"));
    }
}
