//! The listener: plain HTTP/1.1, one request per connection. The page and its fonts, then a JSON API behind a sign-in
//! with the PIN. What the API changes, the session does (`app::webconfig`); this only carries it there and back.

use super::{IDLE, PORT};
use crate::app::webconfig::{Answer, Call, InputPatch, RoleName};
use crate::app::UiAction;
use crate::remote_control::server::local;
use r3cui_sdk::api::{PairError, Pairer, TargetKind};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, watch, Mutex};

const PAGE: &str = include_str!("page.html");
const FONTS: [(&str, &[u8]); 3] = [
    ("/fonts/Inter-400.ttf", include_bytes!("../../ui/assets/fonts/Inter-400.ttf")),
    ("/fonts/Inter-500.ttf", include_bytes!("../../ui/assets/fonts/Inter-500.ttf")),
    ("/fonts/JetBrainsMono-500.ttf", include_bytes!("../../ui/assets/fonts/JetBrainsMono-500.ttf")),
];
const MAX_HEAD: usize = 16 << 10;
const MAX_BODY: usize = 64 << 10;
const READ_WITHIN: Duration = Duration::from_secs(10);
/// Asking a driver for its targets or to start pairing goes out to the device.
const DEVICE_WITHIN: Duration = Duration::from_secs(30);
const MISSES_ALLOWED: u32 = 5;
const LOCKOUT: Duration = Duration::from_secs(60);
const TOKEN_BYTES: usize = 16;

/// Everything one PIN's sitting knows: who signed in, pairings under way, when it was last used.
struct Sitting {
    pin: String,
    tokens: HashSet<String>,
    pairings: HashMap<String, Box<dyn Pairer>>,
    misses: u32,
    locked_until: Option<Instant>,
    used: Instant,
}

type Shared = Arc<Mutex<Sitting>>;

/// Listens while there is a PIN, stops while there is none, and switches off when unused for `IDLE`.
pub(super) async fn run(mut listen: watch::Receiver<Option<String>>, actions: mpsc::UnboundedSender<UiAction>) {
    loop {
        let pin = listen.borrow_and_update().clone();
        let Some(pin) = pin else {
            if listen.changed().await.is_err() {
                return;
            }
            continue;
        };
        let addr = SocketAddr::from(([0, 0, 0, 0], PORT));
        let listener = match TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                tracing::error!("web configurator: cannot listen on {addr}: {e}");
                if listen.changed().await.is_err() {
                    return;
                }
                continue;
            }
        };
        tracing::info!("web configurator: listening on {addr}");
        let sitting: Shared = Arc::new(Mutex::new(Sitting { pin, tokens: HashSet::new(), pairings: HashMap::new(), misses: 0, locked_until: None, used: Instant::now() }));
        let mut idle = tokio::time::interval(Duration::from_secs(60));
        loop {
            tokio::select! {
                changed = listen.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    tracing::info!("web configurator: stopped listening");
                    break;
                }
                _ = idle.tick() => {
                    if sitting.lock().await.used.elapsed() >= IDLE {
                        tracing::info!("web configurator: unused for {}min, switching off", IDLE.as_secs() / 60);
                        let _ = actions.send(UiAction::WebOff);
                    }
                }
                accepted = listener.accept() => {
                    let Ok((stream, peer)) = accepted else { continue };
                    if !local(peer.ip()) {
                        tracing::warn!("web configurator: refused {peer}: not on the local network");
                        continue;
                    }
                    let (sitting, actions) = (sitting.clone(), actions.clone());
                    tokio::spawn(async move {
                        if let Err(e) = connection(stream, sitting, actions).await {
                            tracing::info!("web configurator: {peer}: {e}");
                        }
                    });
                }
            }
        }
    }
}

struct Request {
    method: String,
    path: String,
    query: String,
    token: Option<String>,
    body: Vec<u8>,
}

struct Response {
    status: u16,
    kind: &'static str,
    body: Vec<u8>,
}

impl Response {
    fn json(status: u16, value: &impl serde::Serialize) -> Self {
        Self { status, kind: "application/json", body: serde_json::to_vec(value).unwrap_or_default() }
    }

    fn error(status: u16, why: impl Into<String>) -> Self {
        Self::json(status, &json!({ "error": why.into() }))
    }
}

async fn connection(mut stream: TcpStream, sitting: Shared, actions: mpsc::UnboundedSender<UiAction>) -> anyhow::Result<()> {
    let request = tokio::time::timeout(READ_WITHIN, read(&mut stream)).await??;
    let response = match request {
        Some(r) => route(r, &sitting, &actions).await,
        None => Response::error(400, "Bad request"),
    };
    let reason = match response.status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        429 => "Too Many Requests",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        response.status,
        response.kind,
        response.body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&response.body).await?;
    Ok(())
}

async fn read(stream: &mut TcpStream) -> anyhow::Result<Option<Request>> {
    let mut raw = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if raw.len() > MAX_HEAD {
            return Ok(None);
        }
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(None);
        }
        raw.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&raw[..head_end]).to_string();
    let mut lines = head.lines();
    let mut first = lines.next().unwrap_or_default().split_whitespace();
    let (method, target) = (first.next().unwrap_or_default().to_string(), first.next().unwrap_or_default().to_string());
    let header = |name: &str| head.lines().skip(1).filter_map(|l| l.split_once(':')).find(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_string());
    let length: usize = header("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    if length > MAX_BODY {
        return Ok(None);
    }
    let mut body = raw[head_end..].to_vec();
    while body.len() < length {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(length);
    let (path, query) = target.split_once('?').map_or((target.clone(), String::new()), |(p, q)| (p.to_string(), q.to_string()));
    let token = header("authorization").and_then(|v| v.strip_prefix("Bearer ").map(str::to_string));
    Ok(Some(Request { method, path, query, token, body }))
}

async fn route(r: Request, sitting: &Shared, actions: &mpsc::UnboundedSender<UiAction>) -> Response {
    if r.method == "GET" && r.path == "/" {
        return Response { status: 200, kind: "text/html; charset=utf-8", body: PAGE.as_bytes().to_vec() };
    }
    if r.method == "GET" {
        if let Some((_, font)) = FONTS.iter().find(|(path, _)| *path == r.path) {
            return Response { status: 200, kind: "font/ttf", body: font.to_vec() };
        }
    }
    if !r.path.starts_with("/api/") {
        return Response::error(404, "Not found");
    }
    if r.method == "POST" && r.path == "/api/login" {
        return login(&r.body, sitting).await;
    }
    {
        let mut s = sitting.lock().await;
        if !r.token.as_ref().is_some_and(|t| s.tokens.contains(t)) {
            return Response::error(401, "Sign in with the PIN shown on the remote");
        }
        s.used = Instant::now();
    }
    let segments: Vec<&str> = r.path.trim_start_matches("/api/").split('/').collect();
    let body = || serde_json::from_slice::<Value>(&r.body).unwrap_or(Value::Null);
    match (r.method.as_str(), segments.as_slice()) {
        ("GET", ["config"]) => ask(actions, Call::Config).await,
        ("GET", ["targets"]) => targets(&r.query, actions).await,
        ("POST", ["integrations"]) => match body()["kind"].as_str() {
            Some(kind) => ask(actions, Call::Add(kind.to_string())).await,
            None => Response::error(400, "Say which kind to add"),
        },
        ("PUT", ["integrations", id]) => match body()["fields"].as_object() {
            Some(fields) => ask(actions, Call::Update(id.to_string(), fields.clone())).await,
            None => Response::error(400, "Send the fields to change"),
        },
        ("DELETE", ["integrations", id]) => ask(actions, Call::Remove(id.to_string())).await,
        ("POST", ["integrations", id, "pair"]) => pair(id, sitting, actions).await,
        ("POST", ["integrations", id, "pair", "code"]) => code(id, body()["code"].as_str().unwrap_or_default(), sitting, actions).await,
        ("PUT", ["global", role]) => {
            let Some(role) = RoleName::parse(role) else { return Response::error(404, format!("There is no role {role}")) };
            #[derive(Deserialize)]
            struct Bind {
                driver: Option<String>,
                #[serde(default)]
                target: String,
            }
            match serde_json::from_slice::<Bind>(&r.body) {
                Ok(b) => ask(actions, Call::Bind(role, b.driver, b.target)).await,
                Err(e) => Response::error(400, e.to_string()),
            }
        }
        ("PUT", ["inputs", id]) => match serde_json::from_slice::<InputPatch>(&r.body) {
            Ok(patch) => ask(actions, Call::Input(decode(id), patch)).await,
            Err(e) => Response::error(400, e.to_string()),
        },
        _ => Response::error(404, "Not found"),
    }
}

async fn login(body: &[u8], sitting: &Shared) -> Response {
    let pin = serde_json::from_slice::<Value>(body).ok().and_then(|v| v["pin"].as_str().map(str::to_string)).unwrap_or_default();
    let mut s = sitting.lock().await;
    if s.locked_until.is_some_and(|until| Instant::now() < until) {
        return Response::error(429, "Too many wrong PINs. Wait a minute, then try again.");
    }
    if pin != s.pin {
        s.misses += 1;
        if s.misses >= MISSES_ALLOWED {
            (s.misses, s.locked_until) = (0, Some(Instant::now() + LOCKOUT));
        }
        return Response::error(401, "That PIN is not the one on the remote");
    }
    s.misses = 0;
    let mut bytes = [0u8; TOKEN_BYTES];
    if getrandom::getrandom(&mut bytes).is_err() {
        return Response::error(500, "No randomness for a sign-in");
    }
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    s.tokens.insert(token.clone());
    s.used = Instant::now();
    Response::json(200, &json!({ "token": token }))
}

/// Has the session answer `call`.
async fn call(actions: &mpsc::UnboundedSender<UiAction>, call: Call) -> Option<Answer> {
    let (reply, answer) = oneshot::channel();
    actions.send(UiAction::Web(call, reply)).ok()?;
    answer.await.ok()
}

async fn ask(actions: &mpsc::UnboundedSender<UiAction>, c: Call) -> Response {
    match call(actions, c).await {
        Some(Answer::Config(config)) => Response::json(200, &config),
        Some(Answer::Done | Answer::Driver(_)) => Response::json(200, &json!({})),
        Some(Answer::Failed(why)) => Response::error(400, why),
        None => Response::error(503, "r3cui is busy; try again"),
    }
}

async fn driver(actions: &mpsc::UnboundedSender<UiAction>, id: &str) -> Result<Arc<dyn r3cui_sdk::api::Driver>, Response> {
    match call(actions, Call::Driver(id.to_string())).await {
        Some(Answer::Driver(d)) => Ok(d),
        Some(Answer::Failed(why)) => Err(Response::error(404, why)),
        _ => Err(Response::error(503, "r3cui is busy; try again")),
    }
}

async fn targets(query: &str, actions: &mpsc::UnboundedSender<UiAction>) -> Response {
    let param = |name: &str| query.split('&').filter_map(|p| p.split_once('=')).find(|(k, _)| *k == name).map(|(_, v)| decode(v));
    let (Some(id), Some(kind)) = (param("driver"), param("kind")) else { return Response::error(400, "Say which driver and kind") };
    let Ok(kind) = serde_json::from_value::<TargetKind>(Value::String(kind.clone())) else { return Response::error(400, format!("There is no target kind {kind}")) };
    let d = match driver(actions, &id).await {
        Ok(d) => d,
        Err(r) => return r,
    };
    let Ok(found) = tokio::time::timeout(DEVICE_WITHIN, d.targets(kind)).await else { return Response::error(504, "The integration did not answer in time") };
    let _ = call(actions, Call::Targets { driver: id, kind, targets: found.clone() }).await;
    Response::json(200, &found)
}

async fn pair(id: &str, sitting: &Shared, actions: &mpsc::UnboundedSender<UiAction>) -> Response {
    let d = match driver(actions, id).await {
        Ok(d) => d,
        Err(r) => return r,
    };
    let Some(start) = d.pair() else { return Response::error(400, "This integration does not pair") };
    match tokio::time::timeout(DEVICE_WITHIN, start).await {
        Ok(Ok(pairer)) => {
            sitting.lock().await.pairings.insert(id.to_string(), pairer);
            Response::json(200, &json!({}))
        }
        Ok(Err(e)) => Response::error(400, failure(e)),
        Err(_) => Response::error(504, "The device did not answer in time"),
    }
}

async fn code(id: &str, code: &str, sitting: &Shared, actions: &mpsc::UnboundedSender<UiAction>) -> Response {
    let Some(mut pairer) = sitting.lock().await.pairings.remove(id) else { return Response::error(400, "Start pairing first") };
    match pairer.finish(code).await {
        Ok(fields) => ask(actions, Call::Paired(id.to_string(), fields)).await,
        Err(e) => {
            let wrong = matches!(e, PairError::WrongCode);
            if wrong {
                sitting.lock().await.pairings.insert(id.to_string(), pairer);
            }
            Response::error(400, failure(e))
        }
    }
}

fn failure(e: PairError) -> String {
    match e {
        PairError::WrongCode => "Wrong code. Check the screen and try again.".into(),
        PairError::Failed(why) => why,
    }
}

/// `%xx` escapes in a path segment or query value; `+` is a space in a query.
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(b) => {
                    out.push(b);
                    i += 3;
                    continue;
                }
                None => out.push(b'%'),
            },
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_are_decoded() {
        assert_eq!(decode("media_player.amp"), "media_player.amp");
        assert_eq!(decode("HDMI%201"), "HDMI 1");
        assert_eq!(decode("a+b"), "a b");
        assert_eq!(decode("50%"), "50%");
    }
}
