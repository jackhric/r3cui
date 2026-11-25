//! `FakeKodi`: a Kodi JSON-RPC WebSocket server in this process, for tests that run the real client against it.
//! Shapes follow Kodi 21's. Simplifications: the GUI is one movie list (titles, sorted by sort title) that Up and Down
//! walk as a single column; every movie has the same one audio stream and two subtitle streams; the clock is
//! `tokio::time`'s, so tests may pause time.

use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;

use crate::config::KodiConfig;
use futures_util::{SinkExt, StreamExt};

const PLAYER: u64 = 1;
const NOT_FOUND: i32 = -32601;
const INVALID: i32 = -32602;
const FAILED: i32 = -32100;

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

fn sorted(method: &str, ascending: bool) -> Vec<&'static Movie> {
    let mut list: Vec<_> = LIBRARY.iter().collect();
    match method {
        "year" => list.sort_by_key(|m| m.year),
        "time" => list.sort_by_key(|m| m.runtime),
        "title" => list.sort_by_key(|m| m.title.to_lowercase()),
        _ => list.sort_by_key(|m| m.sort_title()),
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
    json!({ "jsonrpc": "2.0", "method": method, "params": { "data": data, "sender": "xbmc" } }).to_string()
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
        out.push(notification("Player.OnPlay", p.event()));
        out.push(notification("Player.OnAVStart", p.event()));
        self.playing = Some(p);
    }

    fn stop(&mut self, out: &mut Vec<String>) -> bool {
        let Some(p) = self.playing.take() else { return false };
        out.push(notification("Player.OnStop", json!({ "end": false, "item": { "id": p.movie.id, "type": "movie" } })));
        true
    }

    fn set_speed(&mut self, speed: i32, out: &mut Vec<String>) {
        let Some(p) = self.playing.as_mut() else { return };
        let was = p.speed;
        p.rebase(p.position(), speed);
        match (was, speed) {
            (a, b) if a == b => {}
            (_, 0) => out.push(notification("Player.OnPause", p.event())),
            (0, _) => out.push(notification("Player.OnResume", p.event())),
            _ => out.push(notification("Player.OnSpeedChanged", p.event())),
        }
    }

    fn seek(&mut self, to: Duration, out: &mut Vec<String>) {
        let Some(p) = self.playing.as_mut() else { return };
        let offset = to.as_secs_f64() - p.position().as_secs_f64();
        p.rebase(to.min(p.total()), p.speed);
        let mut data = p.event();
        data["player"]["time"] = time(p.base);
        data["player"]["seekoffset"] = time(Duration::from_secs_f64(offset.abs()));
        out.push(notification("Player.OnSeek", data));
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
        let list = sorted("sorttitle", true);
        match method {
            "JSONRPC.Ping" => Ok(json!("pong")),
            "JSONRPC.Version" => Ok(json!({ "version": { "major": 13, "minor": 5, "patch": 0 } })),
            "Application.GetProperties" => {
                Ok(json!({ "name": "Kodi", "version": { "major": 21, "minor": 3, "revision": "20250217-fake", "tag": "stable" } }))
            }
            "Settings.GetSettingValue" => Ok(json!({ "value": false })),
            "XBMC.GetInfoLabels" => {
                let focused = list[self.focus];
                let label = |name: &str| -> String {
                    match name {
                        "Container.Content" => "movies".into(),
                        "Container.FolderPath" => "videodb://movies/titles/".into(),
                        "Container.SortMethod" => "Title".into(),
                        "Container.SortOrder" => "Ascending".into(),
                        "Container.NumItems" => list.len().to_string(),
                        "Container.CurrentItem" => (self.focus + 1).to_string(),
                        "Container.Column" => "0".into(),
                        "Container.Row" | "Container.Position" => self.focus.to_string(),
                        "ListItem.DBID" => focused.id.to_string(),
                        "ListItem.DBTYPE" => "movie".into(),
                        _ => String::new(),
                    }
                };
                let labels = params["labels"].as_array().ok_or((INVALID, "Invalid params."))?;
                Ok(Value::Object(labels.iter().filter_map(Value::as_str).map(|n| (n.to_string(), json!(label(n)))).collect()))
            }
            "Files.GetDirectory" => {
                if params["directory"] != "videodb://movies/titles/" {
                    return Err((FAILED, "Failed to execute method."));
                }
                let sort = &params["sort"];
                let files: Vec<_> = sorted(sort["method"].as_str().unwrap_or("sorttitle"), sort["order"] != "descending").iter().map(|m| m.file()).collect();
                Ok(json!({ "files": files, "limits": { "start": 0, "end": files.len(), "total": files.len() } }))
            }
            "Input.Up" | "Input.Down" | "Input.Left" | "Input.Right" => {
                let last = list.len() - 1;
                match method {
                    "Input.Up" | "Input.Left" => self.focus = self.focus.saturating_sub(1),
                    _ => self.focus = (self.focus + 1).min(last),
                }
                ok()
            }
            "Input.Select" => {
                if self.playing.is_none() {
                    self.start(list[self.focus], false, out);
                }
                ok()
            }
            "Input.Back" | "Input.Home" | "Input.ContextMenu" | "Input.Info" | "Input.ShowOSD" => ok(),
            "Input.ExecuteAction" => {
                match params["action"].as_str() {
                    Some("playpause") => match self.playing.as_ref().map(|p| p.speed) {
                        Some(speed) => self.set_speed(i32::from(speed == 0), out),
                        None => self.start(list[self.focus], false, out),
                    },
                    Some("stop") => {
                        self.stop(out);
                    }
                    Some(_) => {}
                    None => return Err((INVALID, "Invalid params.")),
                }
                ok()
            }
            "Player.GetActivePlayers" => Ok(match self.playing {
                Some(_) => json!([{ "playerid": PLAYER, "playertype": "internal", "type": "video" }]),
                None => json!([]),
            }),
            "Player.GetItem" => {
                let p = self.player(params)?;
                let m = p.movie;
                Ok(json!({ "item": {
                    "id": m.id, "type": "movie", "label": m.title, "title": m.title, "year": m.year,
                    "runtime": m.runtime, "art": m.art(), "file": format!("/movies/{}.mkv", m.id),
                } }))
            }
            "Player.GetProperties" => {
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
            "Player.Open" => {
                let id = params["item"]["movieid"].as_u64().ok_or((INVALID, "Invalid params."))?;
                let movie = movie(id as u32).ok_or((FAILED, "Failed to execute method."))?;
                self.start(movie, params["options"]["resume"] == true, out);
                ok()
            }
            "Player.PlayPause" => {
                let speed = self.player(params)?.speed;
                let to = match &params["play"] {
                    Value::Bool(on) => i32::from(*on),
                    _ => i32::from(speed == 0),
                };
                self.set_speed(to, out);
                Ok(json!({ "speed": to }))
            }
            "Player.Stop" => {
                self.player(params)?;
                self.stop(out);
                ok()
            }
            "Player.Seek" => {
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
            "Player.SetAudioStream" => {
                self.player(params)?;
                let stream = params["stream"].as_u64().filter(|i| *i < 1).ok_or((INVALID, "Invalid params."))?;
                self.playing.as_mut().expect("checked").audio = stream as u32;
                ok()
            }
            "Player.SetSubtitle" => {
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
    notes: broadcast::Sender<String>,
    kick: watch::Sender<u64>,
    calls: Mutex<Vec<String>>,
    open: AtomicUsize,
    accepted: AtomicUsize,
}

impl Shared {
    fn publish(&self, notes: Vec<String>) {
        for n in notes {
            let _ = self.notes.send(n);
        }
    }

    fn respond(&self, frame: &str) -> Option<String> {
        let request: Value = serde_json::from_str(frame).ok()?;
        let id = request.get("id")?.clone();
        let method = request["method"].as_str().unwrap_or_default();
        self.calls.lock().unwrap().push(method.to_string());
        let mut out = Vec::new();
        let result = self.inner.lock().unwrap().handle(method, &request["params"], &mut out);
        let reply = match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
        };
        self.publish(out);
        Some(reply.to_string())
    }
}

struct OpenGuard(Arc<Shared>);

impl Drop for OpenGuard {
    fn drop(&mut self) {
        self.0.open.fetch_sub(1, Ordering::SeqCst);
    }
}

async fn serve(stream: TcpStream, shared: Arc<Shared>, mut notes: broadcast::Receiver<String>, mut kicked: watch::Receiver<u64>) {
    let _open = OpenGuard(shared.clone());
    let Ok(socket) = tokio_tungstenite::accept_async(stream).await else { return };
    let (mut sink, mut source) = socket.split();
    loop {
        let send = tokio::select! {
            frame = source.next() => match frame {
                Some(Ok(Message::Text(text))) => shared.respond(&text),
                Some(Ok(_)) => None,
                _ => break,
            },
            note = notes.recv() => match note {
                Ok(n) => Some(n),
                Err(broadcast::error::RecvError::Lagged(_)) => None,
                Err(_) => break,
            },
            _ = kicked.changed() => break,
        };
        if let Some(text) = send {
            if sink.send(Message::Text(text)).await.is_err() {
                break;
            }
        }
    }
    let _ = sink.close().await;
}

pub struct FakeKodi {
    port: u16,
    shared: Arc<Shared>,
    accept: JoinHandle<()>,
}

impl FakeKodi {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind a loopback port");
        let port = listener.local_addr().expect("bound").port();
        let (notes, _) = broadcast::channel(64);
        let shared = Arc::new(Shared {
            inner: Mutex::default(),
            notes,
            kick: watch::channel(0).0,
            calls: Mutex::default(),
            open: AtomicUsize::new(0),
            accepted: AtomicUsize::new(0),
        });
        let accept = tokio::spawn({
            let shared = shared.clone();
            async move {
                while let Ok((stream, _)) = listener.accept().await {
                    shared.open.fetch_add(1, Ordering::SeqCst);
                    shared.accepted.fetch_add(1, Ordering::SeqCst);
                    tokio::spawn(serve(stream, shared.clone(), shared.notes.subscribe(), shared.kick.subscribe()));
                }
            }
        });
        Self { port, shared, accept }
    }

    /// A config pointing the real service here; the HTTP port is unused, as no artwork is served.
    pub fn config(&self) -> KodiConfig {
        KodiConfig { host: "127.0.0.1".into(), http_port: 1, ws_port: self.port, credentials: None }
    }

    /// Method names received, in order, across all connections.
    pub fn calls(&self) -> Vec<String> {
        self.shared.calls.lock().unwrap().clone()
    }

    /// Connections open now.
    pub fn connections(&self) -> usize {
        self.shared.open.load(Ordering::SeqCst)
    }

    /// Connections ever accepted: tells a reconnect from the first connection.
    pub fn accepted(&self) -> usize {
        self.shared.accepted.load(Ordering::SeqCst)
    }

    /// Start a movie from its beginning, as if from Kodi's own GUI (notifying every connection).
    pub fn play(&self, movieid: u32) {
        let movie = movie(movieid).expect("a movie in the fake library");
        let mut out = Vec::new();
        self.shared.inner.lock().unwrap().start(movie, false, &mut out);
        self.shared.publish(out);
    }

    pub fn stop(&self) {
        let mut out = Vec::new();
        self.shared.inner.lock().unwrap().stop(&mut out);
        self.shared.publish(out);
    }

    /// Drop every connection; the listener stays up, so a client can reconnect.
    pub fn kick(&self) {
        self.shared.kick.send_modify(|generation| *generation += 1);
    }
}

impl Drop for FakeKodi {
    fn drop(&mut self) {
        self.accept.abort();
        self.kick();
    }
}
