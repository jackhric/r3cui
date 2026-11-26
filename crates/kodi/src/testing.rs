//! `FakeKodi`: a Kodi JSON-RPC WebSocket server in this process, for tests that run the real client against it.
//! Shapes follow Kodi 21's. Simplifications: the GUI is one movie list (titles, sorted by sort title) that Up and Down
//! walk as a single column; every movie has the same one audio stream and two subtitle streams; the clock is
//! `tokio::time`'s, so tests may pause time.

use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;
use wsclient::testing::{Broadcaster, FakeServer, Peer, Reply as Frames};

use crate::browse::MOVIES;
use crate::config::KodiConfig;
use crate::methods::gui::{GetInfoLabels, InfoLabel};
use crate::methods::input::{Back, ContextMenu, Down, ExecuteAction, Home, Info, InputAction, Left, Right, Select, ShowOsd, Up};
use crate::methods::library::{GetDirectory, SortMethod, SortOrder};
use crate::methods::player::{GetActivePlayers, GetItem, GetProperties, Open, PlayPause, Seek, SetAudioStream, SetSubtitle, Stop};
use crate::methods::system::{GetApplicationProperties, GetSettingValue, Ping, Version};
use crate::notification::name;
use crate::rpc::{Method, JSONRPC_VERSION};

const PLAYER: u64 = 1;
const NOT_FOUND: i32 = -32601;
const INVALID: i32 = -32602;
const FAILED: i32 = -32100;
const SENDER: &str = "xbmc";
const MOVIES_PATH: &str = "videodb://movies/titles/";

struct Movie {
    id: u32,
    title: &'static str,
    year: u32,
    runtime: u32,
    resume: u32,
}

const LIBRARY: [Movie; 6] = [
    Movie { id: 39, title: "2001: A Space Odyssey", year: 1968, runtime: 8931, resume: 6837 },
    Movie { id: 12, title: "Blade Runner", year: 1982, runtime: 7020, resume: 0 },
    Movie { id: 7, title: "The Godfather", year: 1972, runtime: 10500, resume: 0 },
    Movie { id: 21, title: "Alien", year: 1979, runtime: 7020, resume: 0 },
    Movie { id: 55, title: "A Bronx Tale", year: 1993, runtime: 7260, resume: 0 },
    Movie { id: 3, title: "Arrival", year: 2016, runtime: 6960, resume: 0 },
];

impl Movie {
    fn sort_title(&self) -> String {
        let t = self.title.to_lowercase();
        ["the ", "a ", "an "].iter().find_map(|a| t.strip_prefix(a)).unwrap_or(&t).to_string()
    }

    fn art(&self) -> Value {
        let image = |kind: &str| format!("image://http%3a%2f%2fart.test%2f{}%2f{kind}.jpg/", self.id);
        json!({ "poster": image("poster"), "fanart": image("fanart"), "thumb": image("thumb") })
    }

    fn file(&self) -> Value {
        json!({
            "id": self.id, "type": "movie", "label": self.title, "title": self.title, "year": self.year,
            "runtime": self.runtime, "art": self.art(), "rating": 8.1, "genre": ["Drama"], "plot": "A plot.",
            "resume": { "position": self.resume, "total": if self.resume > 0 { self.runtime } else { 0 } },
            "playcount": 0, "mpaa": "PG", "file": format!("/movies/{}.mkv", self.id),
        })
    }
}

fn movie(id: u32) -> Option<&'static Movie> {
    LIBRARY.iter().find(|m| m.id == id)
}

fn serialized_as<T: serde::Serialize>(value: &Value, expected: T) -> bool {
    *value == serde_json::to_value(expected).expect("serialises")
}

fn sorted(method: &Value, ascending: bool) -> Vec<&'static Movie> {
    let mut list: Vec<_> = LIBRARY.iter().collect();
    if serialized_as(method, SortMethod::Year) {
        list.sort_by_key(|m| m.year);
    } else if serialized_as(method, SortMethod::Duration) {
        list.sort_by_key(|m| m.runtime);
    } else if serialized_as(method, SortMethod::Title) {
        list.sort_by_key(|m| m.title.to_lowercase());
    } else {
        list.sort_by_key(|m| m.sort_title());
    }
    if !ascending {
        list.reverse();
    }
    list
}

fn time(d: Duration) -> Value {
    let ms = d.as_millis() as u64;
    json!({ "hours": ms / 3_600_000, "minutes": ms / 60_000 % 60, "seconds": ms / 1000 % 60, "milliseconds": ms % 1000 })
}

fn audio_streams() -> Value {
    json!([{ "index": 0, "name": "DTS-HD MA", "language": "eng", "codec": "dtshd_ma", "channels": 6, "bitrate": 3_000_000,
        "isdefault": true, "isoriginal": true, "isimpaired": false }])
}

fn subtitle_streams() -> Value {
    json!([
        { "index": 0, "name": "English", "language": "eng", "isforced": false, "isimpaired": false, "isdefault": false },
        { "index": 1, "name": "English SDH", "language": "eng", "isforced": false, "isimpaired": true, "isdefault": false },
    ])
}

fn notification(method: &str, data: Value) -> String {
    json!({ "jsonrpc": JSONRPC_VERSION, "method": method, "params": { "data": data, "sender": SENDER } }).to_string()
}

struct Playing {
    movie: &'static Movie,
    base: Duration,
    since: Instant,
    speed: i32,
    audio: u32,
    subtitle: u32,
    subtitles_on: bool,
}

impl Playing {
    fn total(&self) -> Duration {
        Duration::from_secs(self.movie.runtime.into())
    }

    fn position(&self) -> Duration {
        let ran = self.since.elapsed().as_secs_f64() * f64::from(self.speed);
        Duration::from_secs_f64((self.base.as_secs_f64() + ran).clamp(0.0, self.total().as_secs_f64()))
    }

    fn rebase(&mut self, at: Duration, speed: i32) {
        (self.base, self.since, self.speed) = (at, Instant::now(), speed);
    }

    fn event(&self) -> Value {
        json!({ "item": { "id": self.movie.id, "type": "movie" }, "player": { "playerid": PLAYER, "speed": self.speed } })
    }
}

#[derive(Default)]
struct Inner {
    focus: usize,
    playing: Option<Playing>,
}

type Reply = Result<Value, (i32, &'static str)>;

impl Inner {
    fn start(&mut self, movie: &'static Movie, resume: bool, out: &mut Vec<String>) {
        self.stop(out);
        let base = Duration::from_secs(if resume { movie.resume } else { 0 }.into());
        let p = Playing { movie, base, since: Instant::now(), speed: 1, audio: 0, subtitle: 0, subtitles_on: false };
        out.push(notification(name::PLAYER_ON_PLAY, p.event()));
        out.push(notification(name::PLAYER_ON_AV_START, p.event()));
        self.playing = Some(p);
    }

    fn stop(&mut self, out: &mut Vec<String>) -> bool {
        let Some(p) = self.playing.take() else { return false };
        out.push(notification(name::PLAYER_ON_STOP, json!({ "end": false, "item": { "id": p.movie.id, "type": "movie" } })));
        true
    }

    fn set_speed(&mut self, speed: i32, out: &mut Vec<String>) {
        let Some(p) = self.playing.as_mut() else { return };
        let was = p.speed;
        p.rebase(p.position(), speed);
        match (was, speed) {
            (a, b) if a == b => {}
            (_, 0) => out.push(notification(name::PLAYER_ON_PAUSE, p.event())),
            (0, _) => out.push(notification(name::PLAYER_ON_RESUME, p.event())),
            _ => out.push(notification(name::PLAYER_ON_SPEED_CHANGED, p.event())),
        }
    }

    fn seek(&mut self, to: Duration, out: &mut Vec<String>) {
        let Some(p) = self.playing.as_mut() else { return };
        let offset = to.as_secs_f64() - p.position().as_secs_f64();
        p.rebase(to.min(p.total()), p.speed);
        let mut data = p.event();
        data["player"]["time"] = time(p.base);
        data["player"]["seekoffset"] = time(Duration::from_secs_f64(offset.abs()));
        out.push(notification(name::PLAYER_ON_SEEK, data));
    }

    fn player(&self, params: &Value) -> Result<&Playing, (i32, &'static str)> {
        match (&self.playing, params["playerid"].as_u64()) {
            (Some(p), Some(PLAYER)) => Ok(p),
            (_, Some(_)) => Err((FAILED, "Failed to execute method.")),
            _ => Err((INVALID, "Invalid params.")),
        }
    }

    fn handle(&mut self, method: &str, params: &Value, out: &mut Vec<String>) -> Reply {
        let ok = || Ok(json!("OK"));
        let list = sorted(&Value::Null, true);
        match method {
            Ping::NAME => Ok(json!("pong")),
            Version::NAME => Ok(json!({ "version": { "major": 13, "minor": 5, "patch": 0 } })),
            GetApplicationProperties::NAME => {
                Ok(json!({ "name": "Kodi", "version": { "major": 21, "minor": 3, "revision": "20250217-fake", "tag": "stable" } }))
            }
            GetSettingValue::NAME => Ok(json!({ "value": false })),
            GetInfoLabels::NAME => {
                let focused = list[self.focus];
                let position = self.focus.to_string();
                let values = [
                    (InfoLabel::ContainerContent, MOVIES.to_string()),
                    (InfoLabel::ContainerFolderPath, MOVIES_PATH.to_string()),
                    (InfoLabel::ContainerSortMethod, "Title".to_string()),
                    (InfoLabel::ContainerSortOrder, "Ascending".to_string()),
                    (InfoLabel::ContainerNumItems, list.len().to_string()),
                    (InfoLabel::ContainerCurrentItem, (self.focus + 1).to_string()),
                    (InfoLabel::ContainerColumn, "0".to_string()),
                    (InfoLabel::ContainerRow, position.clone()),
                    (InfoLabel::ContainerPosition, position),
                    (InfoLabel::ListItemDbId, focused.id.to_string()),
                    (InfoLabel::ListItemDbType, "movie".to_string()),
                ];
                let label = |name: &str| values.iter().find(|(l, _)| l.name() == name).map(|(_, v)| v.clone()).unwrap_or_default();
                let labels = params["labels"].as_array().ok_or((INVALID, "Invalid params."))?;
                Ok(Value::Object(labels.iter().filter_map(Value::as_str).map(|n| (n.to_string(), json!(label(n)))).collect()))
            }
            GetDirectory::NAME => {
                if params["directory"] != MOVIES_PATH {
                    return Err((FAILED, "Failed to execute method."));
                }
                let sort = &params["sort"];
                let files: Vec<_> = sorted(&sort["method"], !serialized_as(&sort["order"], SortOrder::Descending)).iter().map(|m| m.file()).collect();
                Ok(json!({ "files": files, "limits": { "start": 0, "end": files.len(), "total": files.len() } }))
            }
            Up::NAME | Down::NAME | Left::NAME | Right::NAME => {
                let last = list.len() - 1;
                match method {
                    Up::NAME | Left::NAME => self.focus = self.focus.saturating_sub(1),
                    _ => self.focus = (self.focus + 1).min(last),
                }
                ok()
            }
            Select::NAME => {
                if self.playing.is_none() {
                    self.start(list[self.focus], false, out);
                }
                ok()
            }
            Back::NAME | Home::NAME | ContextMenu::NAME | Info::NAME | ShowOsd::NAME => ok(),
            ExecuteAction::NAME => {
                let action = &params["action"];
                if action.is_null() {
                    return Err((INVALID, "Invalid params."));
                }
                if serialized_as(action, InputAction::PlayPause) {
                    match self.playing.as_ref().map(|p| p.speed) {
                        Some(speed) => self.set_speed(i32::from(speed == 0), out),
                        None => self.start(list[self.focus], false, out),
                    }
                } else if serialized_as(action, InputAction::Stop) {
                    self.stop(out);
                }
                ok()
            }
            GetActivePlayers::NAME => Ok(match self.playing {
                Some(_) => json!([{ "playerid": PLAYER, "playertype": "internal", "type": "video" }]),
                None => json!([]),
            }),
            GetItem::NAME => {
                let p = self.player(params)?;
                let m = p.movie;
                Ok(json!({ "item": {
                    "id": m.id, "type": "movie", "label": m.title, "title": m.title, "year": m.year,
                    "runtime": m.runtime, "art": m.art(), "file": format!("/movies/{}.mkv", m.id),
                } }))
            }
            GetProperties::NAME => {
                let p = self.player(params)?;
                let names = params["properties"].as_array().ok_or((INVALID, "Invalid params."))?;
                let mut out = serde_json::Map::new();
                for name in names.iter().filter_map(Value::as_str) {
                    let v = match name {
                        "speed" => json!(p.speed),
                        "time" => time(p.position()),
                        "totaltime" => time(p.total()),
                        "canseek" => json!(true),
                        "live" => json!(false),
                        "audiostreams" => audio_streams(),
                        "currentaudiostream" => audio_streams()[p.audio as usize].clone(),
                        "subtitles" => subtitle_streams(),
                        "currentsubtitle" => subtitle_streams()[p.subtitle as usize].clone(),
                        "subtitleenabled" => json!(p.subtitles_on),
                        _ => return Err((INVALID, "Invalid params.")),
                    };
                    out.insert(name.to_string(), v);
                }
                Ok(Value::Object(out))
            }
            Open::NAME => {
                let id = params["item"]["movieid"].as_u64().ok_or((INVALID, "Invalid params."))?;
                let movie = movie(id as u32).ok_or((FAILED, "Failed to execute method."))?;
                self.start(movie, params["options"]["resume"] == true, out);
                ok()
            }
            PlayPause::NAME => {
                let speed = self.player(params)?.speed;
                let to = match &params["play"] {
                    Value::Bool(on) => i32::from(*on),
                    _ => i32::from(speed == 0),
                };
                self.set_speed(to, out);
                Ok(json!({ "speed": to }))
            }
            Stop::NAME => {
                self.player(params)?;
                self.stop(out);
                ok()
            }
            Seek::NAME => {
                let p = self.player(params)?;
                let value = &params["value"];
                let to = if value["time"].is_object() {
                    let t = &value["time"];
                    let part = |k: &str| t[k].as_u64().unwrap_or(0);
                    Duration::from_millis(((part("hours") * 60 + part("minutes")) * 60 + part("seconds")) * 1000 + part("milliseconds"))
                } else if let Some(secs) = value["seconds"].as_i64() {
                    Duration::from_secs_f64((p.position().as_secs_f64() + secs as f64).max(0.0))
                } else {
                    return Err((INVALID, "Invalid params."));
                };
                self.seek(to, out);
                let p = self.playing.as_ref().expect("seeking kept the player");
                Ok(json!({ "time": time(p.position()), "totaltime": time(p.total()), "percentage": 0.0 }))
            }
            SetAudioStream::NAME => {
                self.player(params)?;
                let stream = params["stream"].as_u64().filter(|i| *i < 1).ok_or((INVALID, "Invalid params."))?;
                self.playing.as_mut().expect("checked").audio = stream as u32;
                ok()
            }
            SetSubtitle::NAME => {
                self.player(params)?;
                let enable = params["enable"].as_bool().unwrap_or(false);
                let p = self.playing.as_mut().expect("checked");
                match &params["subtitle"] {
                    Value::Number(i) => match i.as_u64().filter(|i| *i < 2) {
                        Some(i) => (p.subtitle, p.subtitles_on) = (i as u32, enable),
                        None => return Err((INVALID, "Invalid params.")),
                    },
                    Value::String(w) if w == "off" => p.subtitles_on = false,
                    Value::String(w) if w == "on" => p.subtitles_on = true,
                    _ => return Err((INVALID, "Invalid params.")),
                }
                ok()
            }
            _ => Err((NOT_FOUND, "Method not found.")),
        }
    }
}

struct Shared {
    inner: Mutex<Inner>,
    calls: Mutex<Vec<String>>,
}

impl Shared {
    fn respond(&self, frame: &str, notes: &Broadcaster<String>) -> Option<String> {
        let request: Value = serde_json::from_str(frame).ok()?;
        let id = request.get("id")?.clone();
        let method = request["method"].as_str().unwrap_or_default();
        self.calls.lock().unwrap().push(method.to_string());
        let mut out = Vec::new();
        let result = self.inner.lock().unwrap().handle(method, &request["params"], &mut out);
        let reply = match result {
            Ok(result) => json!({ "jsonrpc": JSONRPC_VERSION, "id": id, "result": result }),
            Err((code, message)) => json!({ "jsonrpc": JSONRPC_VERSION, "id": id, "error": { "code": code, "message": message } }),
        };
        out.into_iter().for_each(|note| notes.send(note));
        Some(reply.to_string())
    }
}

struct KodiPeer {
    shared: Arc<Shared>,
    notes: Broadcaster<String>,
}

impl Peer<String> for KodiPeer {
    fn frame(&mut self, text: &str) -> Frames {
        self.shared.respond(text, &self.notes).map_or_else(Frames::nothing, Frames::say)
    }

    fn broadcast(&mut self, note: &String) -> Frames {
        Frames::say(note.clone())
    }
}

pub struct FakeKodi {
    server: FakeServer<String>,
    shared: Arc<Shared>,
}

impl FakeKodi {
    pub async fn start() -> Self {
        Self::start_on("127.0.0.1:0").await
    }

    /// On a given address, to run it as a stand-in Kodi for the desktop UI.
    pub async fn start_on(addr: &str) -> Self {
        let shared = Arc::new(Shared { inner: Mutex::default(), calls: Mutex::default() });
        let server = FakeServer::start(addr, {
            let shared = shared.clone();
            move |notes| KodiPeer { shared: shared.clone(), notes }
        })
        .await;
        Self { server, shared }
    }

    /// A config pointing the real service here; the HTTP port is unused, as no artwork is served.
    pub fn config(&self) -> KodiConfig {
        KodiConfig { host: "127.0.0.1".into(), http_port: 1, ws_port: self.server.addr().port(), credentials: None }
    }

    /// Method names received, in order, across all connections.
    pub fn calls(&self) -> Vec<String> {
        self.shared.calls.lock().unwrap().clone()
    }

    /// Connections open now.
    pub fn connections(&self) -> usize {
        self.server.open()
    }

    /// Connections ever accepted: tells a reconnect from the first connection.
    pub fn accepted(&self) -> usize {
        self.server.accepted()
    }

    /// Start a movie from its beginning, as if from Kodi's own GUI (notifying every connection).
    pub fn play(&self, movieid: u32) {
        let movie = movie(movieid).expect("a movie in the fake library");
        let mut out = Vec::new();
        self.shared.inner.lock().unwrap().start(movie, false, &mut out);
        self.publish(out);
    }

    pub fn stop(&self) {
        let mut out = Vec::new();
        self.shared.inner.lock().unwrap().stop(&mut out);
        self.publish(out);
    }

    /// Drop every connection; the listener stays up, so a client can reconnect.
    pub fn kick(&self) {
        self.server.kick();
    }

    fn publish(&self, notes: Vec<String>) {
        notes.into_iter().for_each(|note| self.server.broadcast(note));
    }
}
