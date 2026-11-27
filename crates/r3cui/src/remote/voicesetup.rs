//! r3cui's voice integration on the remote, set up in one go: install the relay that r3cui's archive carries (or bring
//! it up to this version), set it up so the core has an instance of it, add its voice assistant entity, then turn the
//! microphone on and make that entity the remote's voice assistant. Every step is safe to repeat.

use crate::app::software::Access;
use crate::remote::rest::{Rest, RestError};
use crate::remote::software::{grant, revoke, rest};
use coreapi::{Core, VoiceAssistantUpdate, VoiceControlUpdate};
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use voicelink::{RELAY_DRIVER_ID, RELAY_ENTITY_ID};

/// Next to the binary in r3cui's archive; `packaging/pack.sh` puts it there.
const ARCHIVE: &str = "r3cui-voice.tar.gz";
/// The relay is built from the same workspace, so it carries r3cui's version.
const VERSION: &str = env!("CARGO_PKG_VERSION");
const INSTALL_PATH: &str = "/api/intg/install";
const UPDATE: (&str, &str) = ("update", "true");
const SETUP_PATH: &str = "/api/intg/setup";
const INSTANCES_PATH: &str = "/api/intg/instances";
/// The relay has nothing to ask, so setup ends at once; this is ample.
const SETUP_WITHIN: Duration = Duration::from_secs(30);
const SETUP_POLL: Duration = Duration::from_millis(500);

/// What the System page's voice tile shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum State {
    Ready,
    /// The microphone is off, or the voice assistant is someone else's or none.
    Absent,
}

pub(crate) fn entity_of(integration_id: &str) -> String {
    format!("{integration_id}.{RELAY_ENTITY_ID}")
}

fn is_relay(entity_id: &str) -> bool {
    entity_id.split('.').next() == Some(RELAY_DRIVER_ID)
}

pub(crate) async fn state(core: &Core) -> Result<State, String> {
    let cfg = core.get_voice_control_cfg().await.map_err(|e| e.to_string())?;
    let relay = cfg.voice_assistant.active.is_some_and(|a| is_relay(a.entity_id.as_str()));
    Ok(if cfg.microphone && relay { State::Ready } else { State::Absent })
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
    if path.is_file() { Ok(path) } else { Err("This build of r3cui does not carry the voice integration".into()) }
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
struct Setup {
    state: String,
    #[serde(default)]
    error: Option<String>,
}

/// The REST half, blocking: returns the voice assistant's entity id.
fn set_up(rest: &Rest, archive: &std::path::Path) -> Result<String, RestError> {
    match rest.get::<Driver>(&format!("/api/intg/drivers/{RELAY_DRIVER_ID}"))? {
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
    let _: serde_json::Value = rest.post(&format!("{INSTANCES_PATH}/{integration}/entities"), &json!([RELAY_ENTITY_ID]))?;
    Ok(entity_of(&integration))
}

fn instance(rest: &Rest) -> Result<Option<String>, RestError> {
    let all: Vec<Instance> = rest.get(INSTANCES_PATH)?.unwrap_or_default();
    Ok(all.into_iter().find(|i| i.driver_id == RELAY_DRIVER_ID).map(|i| i.integration_id))
}

fn run_setup(rest: &Rest) -> Result<(), RestError> {
    let mut setup: Setup = rest.post(SETUP_PATH, &json!({ "driver_id": RELAY_DRIVER_ID, "reconfigure": false }))?;
    let path = format!("{SETUP_PATH}/{RELAY_DRIVER_ID}");
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
        std::fs::write(&path, b"RELAY").unwrap();
        path
    }

    fn calls(http: &FakeHttp) -> Vec<String> {
        http.requests().iter().map(|r| format!("{} {}", r.method, r.target)).collect()
    }

    const DRIVER: &str = "/api/intg/drivers/r3cui_voice";
    const ENTITIES: &str = "/api/intg/instances/r3cui_voice.main/entities";
    const ONE: &str = r#"[{"integration_id":"hass.main","driver_id":"hass"},{"integration_id":"r3cui_voice.main","driver_id":"r3cui_voice"}]"#;

    #[test]
    fn a_fresh_remote_gets_the_relay_installed_set_up_and_its_entity_added() {
        let http = FakeHttp::start(vec![
            Canned::answer("POST", INSTALL_PATH, 201, r#"{"driver_id":"r3cui_voice"}"#),
            Canned::answer("POST", SETUP_PATH, 201, r#"{"id":"r3cui_voice","state":"OK"}"#),
            Canned::ok(INSTANCES_PATH, ONE),
            Canned::answer("POST", ENTITIES, 201, r#"["r3cui_voice.main.voice"]"#),
        ]);
        let rest = Rest::at(vec![format!("http://{}", http.addr())], "1");
        assert_eq!(set_up(&rest, &tarball()).unwrap(), "r3cui_voice.main.voice");
        assert_eq!(calls(&http), [format!("GET {DRIVER}"), format!("POST {INSTALL_PATH}"), format!("GET {INSTANCES_PATH}"), format!("POST {ENTITIES}")]);
        assert_eq!(http.requests()[3].body, br#"["voice"]"#);
    }

    #[test]
    fn an_older_relay_is_updated_in_place_and_a_current_one_left_alone() {
        let routes = |version: &str| {
            vec![
                Canned::ok(DRIVER, format!(r#"{{"driver_id":"r3cui_voice","version":"{version}"}}"#)),
                Canned::answer("POST", INSTALL_PATH, 201, "{}"),
                Canned::ok(INSTANCES_PATH, ONE),
                Canned::answer("POST", ENTITIES, 201, "[]"),
            ]
        };
        let old = FakeHttp::start(routes("0.0.1"));
        set_up(&Rest::at(vec![format!("http://{}", old.addr())], "1"), &tarball()).unwrap();
        assert_eq!(calls(&old)[1], format!("POST {INSTALL_PATH}?update=true"));
        let current = FakeHttp::start(routes(VERSION));
        set_up(&Rest::at(vec![format!("http://{}", current.addr())], "1"), &tarball()).unwrap();
        assert!(!calls(&current).iter().any(|c| c.contains(INSTALL_PATH)), "{:?}", calls(&current));
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
    fn only_the_relays_entity_counts_as_r3cuis_voice() {
        assert!(is_relay(&entity_of("r3cui_voice.main")));
        assert!(!is_relay("hass.main.assist"));
    }
}
