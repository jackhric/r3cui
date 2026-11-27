//! A tiny HTTP server for tests: answers fixed routes and keeps what it was sent.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

pub(crate) struct Canned {
    method: &'static str,
    path: &'static str,
    status: u16,
    body: Vec<u8>,
}

impl Canned {
    pub(crate) fn ok(path: &'static str, body: impl Into<Vec<u8>>) -> Self {
        Self::answer("GET", path, 200, body)
    }

    pub(crate) fn answer(method: &'static str, path: &'static str, status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self { method, path, status, body: body.into() }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Seen {
    pub(crate) method: String,
    /// The path and query.
    pub(crate) target: String,
    headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

impl Seen {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

pub(crate) struct FakeHttp {
    addr: std::net::SocketAddr,
    seen: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeHttp {
    pub(crate) fn start(routes: Vec<Canned>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.set_nonblocking(true).expect("nonblocking");
        let addr = listener.local_addr().expect("addr");
        let (seen, stop) = (Arc::new(Mutex::new(Vec::new())), Arc::new(AtomicBool::new(false)));
        let thread = std::thread::spawn({
            let (seen, stop) = (seen.clone(), stop.clone());
            move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => serve(stream, &routes, &seen),
                        Err(_) => std::thread::sleep(Duration::from_millis(5)),
                    }
                }
            }
        });
        Self { addr, seen, stop, thread: Some(thread) }
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    pub(crate) fn addr(&self) -> std::net::SocketAddr {
        self.addr
    }

    pub(crate) fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

impl Drop for FakeHttp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn serve(mut stream: TcpStream, routes: &[Canned], seen: &Mutex<Vec<Seen>>) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut raw = Vec::new();
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => raw.extend_from_slice(&chunk[..n]),
        }
    };
    let head = String::from_utf8_lossy(&raw[..head_end]).to_string();
    let mut lines = head.lines();
    let mut request_line = lines.next().unwrap_or_default().split_whitespace();
    let (method, target) = (request_line.next().unwrap_or_default().to_string(), request_line.next().unwrap_or_default().to_string());
    let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
    let length: usize = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
    let mut body = raw[head_end..].to_vec();
    while body.len() < length {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => body.extend_from_slice(&chunk[..n]),
        }
    }
    let path = target.split('?').next().unwrap_or_default();
    let route = routes.iter().find(|r| r.method == method && r.path == path);
    seen.lock().unwrap().push(Seen { method, target: target.clone(), headers, body });
    let (status, body) = route.map_or((404, Vec::new()), |r| (r.status, r.body.clone()));
    let _ = write!(stream, "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    let _ = stream.write_all(&body);
}
