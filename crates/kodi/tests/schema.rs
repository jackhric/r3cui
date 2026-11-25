//! Every method, parameter, enum value and field the client sends exists in Kodi's own schema (the `JSONRPC.Introspect`
//! output vendored in `schema/`), and every required parameter is sent. Kodi's schema is draft-03 style, so this is a
//! small hand-written checker rather than a JSON-Schema validator.

use kodi::ids::*;
use kodi::methods::{gui::*, input, library::*, player::*, system::*};
use kodi::rpc::Method;
use kodi::wire::Time;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::time::Duration;

fn schema() -> Value {
    serde_json::from_str(include_str!("../schema/kodi-21.3-jsonrpc-13.5.json")).unwrap()
}

/// The enum values of a type, following `extends` and `items`.
fn enums(schema: &Value, ty: &str) -> BTreeSet<String> {
    let t = &schema["types"][ty];
    assert!(!t.is_null(), "no type {ty} in the schema");
    let mut out: BTreeSet<String> = t["enums"].as_array().or(t["items"]["enums"].as_array()).into_iter().flatten().filter_map(|v| v.as_str().map(String::from)).collect();
    let parents = match &t["extends"] {
        Value::String(s) => vec![s.clone()],
        Value::Array(a) => a.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        _ => vec![],
    };
    for p in parents {
        out.extend(enums(schema, &p));
    }
    out
}

fn names<T: Serialize>(values: &[T]) -> Vec<String> {
    values.iter().map(|v| serde_json::to_value(v).unwrap().as_str().unwrap().to_string()).collect()
}

fn check_enum<T: Serialize>(schema: &Value, ty: &str, values: &[T]) {
    let known = enums(schema, ty);
    for v in names(values) {
        assert!(known.contains(&v), "{ty} has no {v:?}");
    }
}

/// The method exists, every parameter sent is one it takes, and every required one is sent.
fn check_method<M: Method>(schema: &Value, m: &M) {
    let def = &schema["methods"][M::NAME];
    assert!(!def.is_null(), "no method {}", M::NAME);
    let params: Vec<&Value> = def["params"].as_array().unwrap().iter().collect();
    let sent = serde_json::to_value(m).unwrap();
    let sent = sent.as_object().unwrap();
    for key in sent.keys() {
        assert!(params.iter().any(|p| p["name"] == key.as_str()), "{} takes no {key:?}", M::NAME);
    }
    for p in params.iter().filter(|p| p["required"] == true) {
        assert!(sent.contains_key(p["name"].as_str().unwrap()), "{} needs {}", M::NAME, p["name"]);
    }
}

#[test]
fn methods_and_parameters() {
    let s = schema();
    let player = PlayerId(1);
    check_method(&s, &GetActivePlayers {});
    check_method(&s, &GetItem { playerid: player, properties: &[ItemField::Title] });
    check_method(&s, &GetProperties { playerid: player, properties: &[PlayerProperty::Time] });
    check_method(&s, &PlayPause { playerid: player, play: Toggle::Toggle });
    check_method(&s, &Stop { playerid: player });
    check_method(&s, &Seek { playerid: player, value: SeekTo::Time { time: Time(Duration::from_secs(5)) } });
    check_method(&s, &SetAudioStream { playerid: player, stream: AudioStreamIndex(0) });
    check_method(&s, &SetSubtitle { playerid: player, subtitle: SubtitleTarget::Word(SubtitleWord::Off), enable: false });
    check_method(&s, &Open { item: OpenItem::Movie { movieid: MovieId(39) }, options: OpenOptions { resume: true } });
    check_method(&s, &input::Up {});
    check_method(&s, &input::Down {});
    check_method(&s, &input::Left {});
    check_method(&s, &input::Right {});
    check_method(&s, &input::Select {});
    check_method(&s, &input::Back {});
    check_method(&s, &input::Home {});
    check_method(&s, &input::ContextMenu {});
    check_method(&s, &input::Info {});
    check_method(&s, &input::ShowOsd {});
    check_method(&s, &input::ExecuteAction { action: input::InputAction::PageUp });
    check_method(&s, &GetInfoLabels::of(&[InfoLabel::ContainerContent]));
    check_method(&s, &GetGuiProperties { properties: &[GuiProperty::CurrentWindow] });
    let sort = Sort { method: SortMethod::SortTitle, order: SortOrder::Ascending, ignorearticle: true };
    check_method(&s, &GetDirectory { directory: "videodb://movies/titles/".into(), media: "video", properties: MOVIE_FIELDS, sort });
    check_method(&s, &Ping {});
    check_method(&s, &Version {});
    check_method(&s, &GetApplicationProperties { properties: &[AppProperty::Name] });
    check_method(&s, &GetSettingValue { setting: "filelists.ignorethewhensorting" });
}

#[test]
fn enum_values() {
    let s = schema();
    use PlayerProperty as P;
    check_enum(&s, "Player.Property.Name", &[P::Speed, P::Time, P::TotalTime, P::CanSeek, P::Live, P::AudioStreams, P::CurrentAudioStream, P::Subtitles, P::CurrentSubtitle, P::SubtitleEnabled]);
    use ItemField as I;
    check_enum(&s, "List.Fields.All", &[I::Title, I::ShowTitle, I::Season, I::Episode, I::Year, I::Runtime, I::Art, I::File]);
    check_enum(&s, "List.Fields.Files", MOVIE_FIELDS);
    check_enum(&s, "List.Fields.Files", SHOW_FIELDS);
    use input::InputAction as A;
    check_enum(&s, "Input.Action", &[A::PageUp, A::PageDown, A::SkipNext, A::SkipPrevious, A::PlayPause, A::Stop, A::Osd, A::ChapterOrBigStepForward, A::ChapterOrBigStepBack]);
    check_enum(&s, "GUI.Property.Name", &[GuiProperty::CurrentWindow, GuiProperty::Fullscreen]);
    check_enum(&s, "Application.Property.Name", &[AppProperty::Name, AppProperty::Version]);
    use SortMethod as M;
    let sorts = enums_of_property(&s, "List.Sort", "method");
    for m in names(&[M::SortTitle, M::Title, M::Year, M::DateAdded, M::Rating, M::UserRating, M::LastPlayed, M::PlayCount, M::Duration, M::Mpaa]) {
        assert!(sorts.contains(&m), "List.Sort.method has no {m:?}");
    }
}

fn enums_of_property(s: &Value, ty: &str, prop: &str) -> BTreeSet<String> {
    s["types"][ty]["properties"][prop]["enums"].as_array().unwrap().iter().filter_map(|v| v.as_str().map(String::from)).collect()
}

#[test]
fn notifications_exist() {
    let s = schema();
    for n in [
        "Player.OnPlay", "Player.OnAVStart", "Player.OnAVChange", "Player.OnPause", "Player.OnResume", "Player.OnSpeedChanged",
        "Player.OnSeek", "Player.OnStop", "Player.OnPropertyChanged", "GUI.OnScreensaverActivated", "GUI.OnScreensaverDeactivated",
        "GUI.OnDPMSActivated", "GUI.OnDPMSDeactivated", "Input.OnInputRequested", "Input.OnInputFinished", "VideoLibrary.OnUpdate",
        "VideoLibrary.OnRemove", "VideoLibrary.OnScanFinished", "VideoLibrary.OnCleanFinished", "VideoLibrary.OnRefresh",
        "System.OnQuit", "System.OnRestart", "System.OnSleep", "System.OnWake",
    ] {
        assert!(!s["notifications"][n].is_null(), "no notification {n}");
    }
}
