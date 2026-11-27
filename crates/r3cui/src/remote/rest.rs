//! The core's REST API, for what the WebSocket cannot do: putting a new UI on the remote and taking this one off, and
//! installing and setting up r3cui's voice integration. All need the web configurator's PIN (basic auth as
//! `web-configurator`).

use base64::Engine as _;
use serde::de::DeserializeOwned;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

const USER: &str = "web-configurator";
const INSTALL_UI_PATH: &str = "/api/system/install/ui";
const BOUNDARY: &str = "r3cui-archive-boundary";
const FILE_FIELD: &str = "file";
/// Any name does: the remote reads the archive, not what it is called.
const UPLOAD_NAME: &str = "archive.tar.gz";
/// The flag is already set on a remote that runs r3cui; the endpoint wants it spelled out all the same.
const ACKNOWLEDGE_WARRANTY: (&str, &str) = ("void_warranty", "yes");
const DISABLE: (&str, &str) = ("enable", "false");
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(300);
const JSON: &str = "application/json";
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RestError {
    /// Nothing answered at any address tried.
    Unreachable(String),
    /// The answer was not what the API says it is.
    Unexpected(String),
    /// The core answered, and said no.
    Refused { status: u16, what: String },
    Local(String),
}

impl fmt::Display for RestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable(why) => write!(f, "the remote's REST API did not answer: {why}"),
            Self::Refused { status: 401, .. } => f.write_str("the remote did not accept the PIN"),
            Self::Refused { status: 403, .. } => f.write_str("the remote did not allow it (403)"),
            Self::Refused { status, what } => write!(f, "the remote refused it ({status} {what})"),
            Self::Unexpected(why) => write!(f, "the remote answered unexpectedly: {why}"),
            Self::Local(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for RestError {}

/// Why one try at one address failed.
enum Attempt {
    Net(Box<ureq::Error>),
    Local(String),
}

impl From<ureq::Error> for Attempt {
    fn from(e: ureq::Error) -> Self {
        Self::Net(Box::new(e))
    }
}

pub(crate) struct Rest {
    bases: Vec<String>,
    auth: String,
}

impl Rest {
    /// For the core at `socket_url` (`UC_SOCKET_URL`): REST is on the same host, on the WebSocket's port, and failing
    /// that on the web's own.
    pub(crate) fn new(socket_url: &str, pin: &str) -> Self {
        Self::at(bases(socket_url), pin)
    }

    pub(crate) fn at(bases: Vec<String>, pin: &str) -> Self {
        let auth = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{USER}:{pin}")));
        Self { bases, auth }
    }

    /// Whether the REST API answers and takes the PIN: asks after the installed UI.
    pub(crate) fn check(&self) -> Result<(), RestError> {
        self.try_each(|base| ureq::get(&format!("{base}{INSTALL_UI_PATH}")).set("Authorization", &self.auth).timeout(CALL_TIMEOUT).call().map_err(Attempt::from)).map(drop)
    }

    /// Installs the archive as the remote's UI. The core restarts the UI when it succeeds, so there may be no answer.
    pub(crate) fn install(&self, archive: &Path) -> Result<(), RestError> {
        self.upload(INSTALL_UI_PATH, Some(ACKNOWLEDGE_WARRANTY), archive).map(drop)
    }

    /// Posts an archive as the form's one file.
    pub(crate) fn upload(&self, path: &str, query: Option<(&str, &str)>, archive: &Path) -> Result<String, RestError> {
        let length = std::fs::metadata(archive).map_err(|e| RestError::Local(format!("{}: {e}", archive.display())))?.len();
        let (head, tail) = (multipart_head(), multipart_tail());
        let total = head.len() as u64 + length + tail.len() as u64;
        let answer = self.try_each(|base| {
            let file = File::open(archive).map_err(|e| Attempt::Local(format!("{}: {e}", archive.display())))?;
            let body = head.as_slice().chain(file).chain(tail.as_slice());
            let mut request = ureq::post(&format!("{base}{path}"));
            if let Some((k, v)) = query {
                request = request.query(k, v);
            }
            request
                .set("Authorization", &self.auth)
                .set("Content-Type", &format!("multipart/form-data; boundary={BOUNDARY}"))
                .set("Content-Length", &total.to_string())
                .timeout(UPLOAD_TIMEOUT)
                .send(body)
                .map_err(Attempt::from)
        })?;
        text(answer)
    }

    /// `None` when the core answers 404: there is no such thing.
    pub(crate) fn get<T: DeserializeOwned>(&self, path: &str) -> Result<Option<T>, RestError> {
        match self.try_each(|base| ureq::get(&format!("{base}{path}")).set("Authorization", &self.auth).timeout(CALL_TIMEOUT).call().map_err(Attempt::from)) {
            Err(RestError::Refused { status: 404, .. }) => Ok(None),
            answer => parse(&text(answer?)?).map(Some),
        }
    }

    pub(crate) fn post<T: DeserializeOwned>(&self, path: &str, body: &serde_json::Value) -> Result<T, RestError> {
        self.send_json("POST", path, body)
    }

    pub(crate) fn put<T: DeserializeOwned>(&self, path: &str, body: &serde_json::Value) -> Result<T, RestError> {
        self.send_json("PUT", path, body)
    }

    fn send_json<T: DeserializeOwned>(&self, method: &str, path: &str, body: &serde_json::Value) -> Result<T, RestError> {
        let body = body.to_string();
        let answer = self.try_each(|base| {
            ureq::request(method, &format!("{base}{path}")).set("Authorization", &self.auth).set("Content-Type", JSON).timeout(CALL_TIMEOUT).send_string(&body).map_err(Attempt::from)
        })?;
        parse(&text(answer)?)
    }

    /// Removes the UI; when the core will not, switches to the stock one and leaves this one installed.
    pub(crate) fn uninstall(&self) -> Result<(), RestError> {
        let removed = self.try_each(|base| ureq::delete(&format!("{base}{INSTALL_UI_PATH}")).set("Authorization", &self.auth).timeout(CALL_TIMEOUT).call().map_err(Attempt::from)).map(drop);
        match removed {
            Err(RestError::Refused { status, what }) if status != 401 => {
                tracing::warn!("removing the UI failed ({status} {what}); switching to the stock one");
                self.try_each(|base| ureq::put(&format!("{base}{INSTALL_UI_PATH}")).query(DISABLE.0, DISABLE.1).set("Authorization", &self.auth).timeout(CALL_TIMEOUT).call().map_err(Attempt::from)).map(drop)
            }
            other => other,
        }
    }

    fn try_each(&self, send: impl Fn(&str) -> Result<ureq::Response, Attempt>) -> Result<ureq::Response, RestError> {
        let mut last = RestError::Unreachable("no address to try".into());
        for base in &self.bases {
            match send(base) {
                Ok(answer) => return Ok(answer),
                Err(Attempt::Local(why)) => return Err(RestError::Local(why)),
                Err(Attempt::Net(e)) => match *e {
                    ureq::Error::Status(status, response) => return Err(RestError::Refused { status, what: response.status_text().to_string() }),
                    ureq::Error::Transport(t) => last = RestError::Unreachable(format!("{base}: {t}")),
                },
            }
        }
        Err(last)
    }
}

fn text(answer: ureq::Response) -> Result<String, RestError> {
    answer.into_string().map_err(|e| RestError::Unexpected(e.to_string()))
}

fn parse<T: DeserializeOwned>(text: &str) -> Result<T, RestError> {
    // Some calls answer with no body at all; to them, nothing is `null`.
    serde_json::from_str(if text.trim().is_empty() { "null" } else { text }).map_err(|e| RestError::Unexpected(e.to_string()))
}

fn multipart_head() -> Vec<u8> {
    format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{FILE_FIELD}\"; filename=\"{UPLOAD_NAME}\"\r\nContent-Type: application/gzip\r\n\r\n").into_bytes()
}

fn multipart_tail() -> Vec<u8> {
    format!("\r\n--{BOUNDARY}--\r\n").into_bytes()
}

/// `http://host:port`, then `http://host`, for a socket URL such as `ws://127.0.0.1:8080/ws`.
fn bases(socket_url: &str) -> Vec<String> {
    let (scheme, rest) = match socket_url.split_once("://") {
        Some(("wss", rest)) => ("https", rest),
        Some((_, rest)) => ("http", rest),
        None => ("http", socket_url),
    };
    let authority = rest.split('/').next().unwrap_or_default();
    let host = match authority.rsplit_once(':') {
        Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) => host,
        _ => authority,
    };
    let mut all = vec![format!("{scheme}://{authority}"), format!("{scheme}://{host}")];
    all.dedup();
    all
}

#[cfg(test)]
mod tests {
    use crate::remote::selfupdate::fake_http::{Canned, FakeHttp};
    use super::*;

    fn rest(http: &FakeHttp) -> Rest {
        Rest::at(vec![format!("http://{}", http.addr())], "4827")
    }

    fn archive(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("r3cui-rest-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn the_core_is_found_by_the_socket_url() {
        assert_eq!(bases("ws://127.0.0.1:8080/ws"), ["http://127.0.0.1:8080", "http://127.0.0.1"]);
        assert_eq!(bases("ws://127.0.0.1/ws"), ["http://127.0.0.1"]);
        assert_eq!(bases("wss://remote.local:8443/ws"), ["https://remote.local:8443", "https://remote.local"]);
    }

    #[test]
    fn an_install_posts_the_archive_with_the_pin() {
        let http = FakeHttp::start(vec![Canned::answer("POST", INSTALL_UI_PATH, 200, "{}")]);
        let file = archive("install.tar.gz", b"ARCHIVE-BYTES");
        rest(&http).install(&file).unwrap();
        let seen = &http.requests()[0];
        assert_eq!((seen.method.as_str(), seen.target.as_str()), ("POST", "/api/system/install/ui?void_warranty=yes"));
        let expected = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode("web-configurator:4827"));
        assert_eq!(seen.header("authorization"), Some(expected.as_str()));
        assert!(seen.header("content-type").unwrap().starts_with("multipart/form-data; boundary="));
        let body = String::from_utf8_lossy(&seen.body);
        assert!(body.contains("name=\"file\"; filename=\"archive.tar.gz\"") && body.contains("ARCHIVE-BYTES") && body.trim_end().ends_with("--"), "{body}");
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn a_wrong_pin_and_a_full_disk_are_told_apart() {
        let http = FakeHttp::start(vec![Canned::answer("POST", INSTALL_UI_PATH, 401, "")]);
        let file = archive("denied.tar.gz", b"x");
        let err = rest(&http).install(&file).unwrap_err();
        assert_eq!(err.to_string(), "the remote did not accept the PIN");
        let http = FakeHttp::start(vec![Canned::answer("POST", INSTALL_UI_PATH, 507, "")]);
        assert!(matches!(rest(&http).install(&file), Err(RestError::Refused { status: 507, .. })));
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn an_unreachable_first_address_falls_back_to_the_next() {
        let http = FakeHttp::start(vec![Canned::answer("DELETE", INSTALL_UI_PATH, 200, "")]);
        let dead = "http://127.0.0.1:1".to_string();
        Rest::at(vec![dead, format!("http://{}", http.addr())], "1").uninstall().unwrap();
        assert_eq!(http.requests()[0].method, "DELETE");
        assert!(matches!(Rest::at(vec!["http://127.0.0.1:1".into()], "1").uninstall(), Err(RestError::Unreachable(_))));
    }

    #[test]
    fn uninstalling_falls_back_to_switching_the_stock_ui_on() {
        let http = FakeHttp::start(vec![Canned::answer("DELETE", INSTALL_UI_PATH, 500, ""), Canned::answer("PUT", INSTALL_UI_PATH, 200, "")]);
        rest(&http).uninstall().unwrap();
        let methods: Vec<_> = http.requests().iter().map(|r| (r.method.clone(), r.target.clone())).collect();
        assert_eq!(methods, [("DELETE".into(), INSTALL_UI_PATH.to_string()), ("PUT".into(), format!("{INSTALL_UI_PATH}?enable=false"))]);
        let denied = FakeHttp::start(vec![Canned::answer("DELETE", INSTALL_UI_PATH, 401, "")]);
        assert!(rest(&denied).uninstall().is_err());
        assert_eq!(denied.requests().len(), 1, "a wrong PIN is not retried as another call");
    }
}
