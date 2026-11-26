//! `FakeTv`: an Android TV for tests and the desktop app. It runs both TLS channels in process: pairing (it shows a
//! code and checks the secret exactly as a TV does) and the remote channel (it only lets paired clients in, pings,
//! records what it is sent, and pushes state on request).

use crate::credentials::Credentials;
use crate::endpoint::Endpoint;
use crate::error::Result;
use crate::protocol::polo::{Configuration, Encoding, Options, OuterMessage, PairingRequestAck, RoleType, SecretAck, Status};
use crate::protocol::remote::{feature, AppInfo, DeviceInfo, ImeKeyInject, PingRequest, RemoteMessage, SetVolumeLevel, Start};
use crate::protocol::{Direction, KeyCode};
use crate::secret::{digest, Code};
use crate::state::Volume;
use crate::tls::provider;
use rustls::client::danger::HandshakeSignatureValid;
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{CertificateError, DigitallySignedStruct, DistinguishedName, ServerConfig, SignatureScheme};
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;

pub const FAKE_VENDOR: &str = "Fake";
pub const FAKE_MODEL: &str = "FakeTV";
pub const FAKE_SERVER_NAME: &str = "FakeTV";
/// The last two bytes of every code the fake shows; the first is the hash's.
const CODE_TAIL: [u8; 2] = [0x0A, 0x5C];
const DEFAULT_PING_EVERY: Duration = Duration::from_secs(1);
const EVENT_BACKLOG: usize = 16;

pub struct FakeOptions {
    pub remote_addr: String,
    pub pairing_addr: String,
    pub ping_every: Duration,
}

impl Default for FakeOptions {
    fn default() -> Self {
        Self { remote_addr: "127.0.0.1:0".into(), pairing_addr: "127.0.0.1:0".into(), ping_every: DEFAULT_PING_EVERY }
    }
}

#[derive(Clone)]
enum Push {
    Message(Box<RemoteMessage>),
    Kick,
}

struct TvScreen {
    on: bool,
    app: String,
    volume: Volume,
}

struct Shared {
    server: Credentials,
    paired: Arc<Mutex<HashSet<Vec<u8>>>>,
    code: watch::Sender<Option<String>>,
    keys: Mutex<Vec<(KeyCode, Direction)>>,
    launches: Mutex<Vec<String>>,
    pings_answered: AtomicUsize,
    accepted: AtomicUsize,
    screen: Mutex<TvScreen>,
    push: broadcast::Sender<Push>,
    ping_every: Duration,
}

pub struct FakeTv {
    endpoint: Endpoint,
    shared: Arc<Shared>,
    tasks: Vec<JoinHandle<()>>,
}

/// The RSA key generation is slow, so every fake in a process shares one certificate.
fn server_credentials() -> Credentials {
    static SERVER: OnceLock<Credentials> = OnceLock::new();
    SERVER.get_or_init(|| Credentials::generate().expect("the fake TV's certificate")).clone()
}

impl FakeTv {
    pub async fn start() -> Self {
        Self::start_with(FakeOptions::default()).await
    }

    pub async fn start_with(options: FakeOptions) -> Self {
        let shared = Arc::new(Shared {
            server: tokio::task::spawn_blocking(server_credentials).await.unwrap(),
            paired: Arc::default(),
            code: watch::channel(None).0,
            keys: Mutex::default(),
            launches: Mutex::default(),
            pings_answered: AtomicUsize::new(0),
            accepted: AtomicUsize::new(0),
            screen: Mutex::new(TvScreen { on: true, app: "com.fake.launcher".into(), volume: Volume { level: 10, max: 100, muted: false } }),
            push: broadcast::channel(EVENT_BACKLOG).0,
            ping_every: options.ping_every,
        });
        let remote = TcpListener::bind(&options.remote_addr).await.expect("bind the fake TV's remote port");
        let pairing = TcpListener::bind(&options.pairing_addr).await.expect("bind the fake TV's pairing port");
        let endpoint = Endpoint::new("127.0.0.1").with_ports(remote.local_addr().unwrap().port(), pairing.local_addr().unwrap().port());
        let tasks = vec![
            tokio::spawn(accept(remote, shared.clone(), Which::Remote)),
            tokio::spawn(accept(pairing, shared.clone(), Which::Pairing)),
        ];
        Self { endpoint, shared, tasks }
    }

    pub fn endpoint(&self) -> Endpoint {
        self.endpoint.clone()
    }

    /// The code the TV is showing, once a client has started pairing.
    pub fn code(&self) -> watch::Receiver<Option<String>> {
        self.shared.code.subscribe()
    }

    pub fn keys(&self) -> Vec<(KeyCode, Direction)> {
        self.shared.keys.lock().unwrap().clone()
    }

    pub fn launches(&self) -> Vec<String> {
        self.shared.launches.lock().unwrap().clone()
    }

    pub fn pings_answered(&self) -> usize {
        self.shared.pings_answered.load(Ordering::SeqCst)
    }

    /// Remote connections that got past the handshake.
    pub fn accepted(&self) -> usize {
        self.shared.accepted.load(Ordering::SeqCst)
    }

    pub fn paired_clients(&self) -> usize {
        self.shared.paired.lock().unwrap().len()
    }

    pub fn forget_clients(&self) {
        self.shared.paired.lock().unwrap().clear();
        self.kick();
    }

    pub fn set_on(&self, on: bool) {
        self.shared.screen.lock().unwrap().on = on;
        self.push(started(on));
    }

    pub fn set_app(&self, package: &str) {
        self.shared.screen.lock().unwrap().app = package.into();
        self.push(app(package));
    }

    pub fn set_volume(&self, volume: Volume) {
        self.shared.screen.lock().unwrap().volume = volume;
        self.push(volume_message(volume));
    }

    /// Hangs up on every connected remote client.
    pub fn kick(&self) {
        let _ = self.shared.push.send(Push::Kick);
    }

    fn push(&self, message: RemoteMessage) {
        let _ = self.shared.push.send(Push::Message(Box::new(message)));
    }
}

impl Drop for FakeTv {
    fn drop(&mut self) {
        self.tasks.iter().for_each(JoinHandle::abort);
        let _ = self.shared.push.send(Push::Kick);
    }
}

fn started(on: bool) -> RemoteMessage {
    RemoteMessage { start: Some(Start { started: on }), ..Default::default() }
}

fn app(package: &str) -> RemoteMessage {
    RemoteMessage { ime_key_inject: Some(ImeKeyInject { app_info: Some(AppInfo { counter: 1, app_package: package.into() }) }), ..Default::default() }
}

fn volume_message(v: Volume) -> RemoteMessage {
    RemoteMessage { set_volume_level: Some(SetVolumeLevel { player_model: FAKE_MODEL.into(), volume_max: v.max, volume_level: v.level, volume_muted: v.muted }), ..Default::default() }
}

#[derive(Clone, Copy)]
enum Which {
    Remote,
    Pairing,
}

async fn accept(listener: TcpListener, shared: Arc<Shared>, kind: Which) {
    let paired_only = matches!(kind, Which::Remote).then(|| shared.paired.clone());
    let acceptor = acceptor(&shared.server, paired_only);
    loop {
        let Ok((tcp, _)) = listener.accept().await else { return };
        let (acceptor, shared) = (acceptor.clone(), shared.clone());
        tokio::spawn(async move {
            let Ok(tls) = acceptor.accept(tcp).await else { return };
            let client = tls.get_ref().1.peer_certificates().and_then(|c| c.first()).map(|c| c.to_vec()).unwrap_or_default();
            let channel = crate::channel::Channel::new(tls);
            let _ = match kind {
                Which::Pairing => pairing(channel, client, &shared).await,
                Which::Remote => remote(channel, &shared).await,
            };
        });
    }
}

async fn pairing<S>(mut channel: crate::channel::Channel<S>, client: Vec<u8>, shared: &Shared) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let server_cert = shared.server.cert_der()?.to_vec();
    channel.recv::<OuterMessage>().await?.pairing_request.ok_or(crate::Error::Unexpected("pairing request"))?;
    let mut ack = OuterMessage::ok();
    ack.pairing_request_ack = Some(PairingRequestAck { server_name: Some(FAKE_SERVER_NAME.into()) });
    channel.send(&ack).await?;

    channel.recv::<OuterMessage>().await?.options.ok_or(crate::Error::Unexpected("options"))?;
    let mut options = OuterMessage::ok();
    options.options = Some(Options { input_encodings: vec![Encoding::hex_code()], output_encodings: vec![], preferred_role: Some(RoleType::Output as i32) });
    channel.send(&options).await?;

    let configuration: Configuration = channel.recv::<OuterMessage>().await?.configuration.ok_or(crate::Error::Unexpected("configuration"))?;
    debug_assert_eq!(configuration.encoding, Encoding::hex_code());
    let mut done = OuterMessage::ok();
    done.configuration_ack = Some(crate::protocol::polo::ConfigurationAck {});

    let expected = digest(&client, &server_cert, CODE_TAIL)?;
    let code = Code { check: expected[0], tail: CODE_TAIL };
    shared.code.send_replace(Some(code.display()));
    channel.send(&done).await?;

    let secret = channel.recv::<OuterMessage>().await?.secret.ok_or(crate::Error::Unexpected("secret"))?.secret;
    if secret == expected {
        shared.paired.lock().unwrap().insert(client);
        let mut ack = OuterMessage::ok();
        ack.secret_ack = Some(SecretAck { secret });
        channel.send(&ack).await
    } else {
        channel.send(&OuterMessage::with_status(Status::BadSecret)).await
    }
}

async fn remote<S>(mut channel: crate::channel::Channel<S>, shared: &Shared) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut pushes = shared.push.subscribe();
    shared.accepted.fetch_add(1, Ordering::SeqCst);
    let device = DeviceInfo { model: FAKE_MODEL.into(), vendor: FAKE_VENDOR.into(), ..Default::default() };
    channel.send(&RemoteMessage::configure(feature::WANTED, device)).await?;
    let mut ping = tokio::time::interval(shared.ping_every);
    let mut pings_sent = 0;
    loop {
        tokio::select! {
            incoming = channel.recv::<RemoteMessage>() => {
                let message = incoming?;
                if message.configure.is_some() {
                    channel.send(&RemoteMessage::set_active(feature::WANTED)).await?;
                }
                if message.set_active.is_some() {
                    let (on, package, volume) = {
                        let screen = shared.screen.lock().unwrap();
                        (screen.on, screen.app.clone(), screen.volume)
                    };
                    for first in [volume_message(volume), app(&package), started(on)] {
                        channel.send(&first).await?;
                    }
                }
                if let Some(key) = message.key_inject {
                    let pair = (KeyCode::try_from(key.key_code), Direction::try_from(key.direction));
                    if let (Ok(code), Ok(direction)) = pair {
                        shared.keys.lock().unwrap().push((code, direction));
                    }
                }
                if let Some(launch) = message.app_link_launch_request {
                    shared.launches.lock().unwrap().push(launch.app_link);
                }
                if message.ping_response.is_some() {
                    shared.pings_answered.fetch_add(1, Ordering::SeqCst);
                }
            }
            _ = ping.tick() => {
                pings_sent += 1;
                let request = RemoteMessage { ping_request: Some(PingRequest { val1: pings_sent, val2: 0 }), ..Default::default() };
                channel.send(&request).await?;
            }
            push = pushes.recv() => match push {
                Ok(Push::Message(message)) => channel.send(&*message).await?,
                Ok(Push::Kick) | Err(broadcast::error::RecvError::Closed) => return Ok(()),
                Err(broadcast::error::RecvError::Lagged(_)) => {}
            },
        }
    }
}

/// Accepts any client certificate for pairing; for the remote channel only the ones that have paired.
#[derive(Debug)]
struct Clients {
    provider: Arc<rustls::crypto::CryptoProvider>,
    paired: Option<Arc<Mutex<HashSet<Vec<u8>>>>>,
}

impl ClientCertVerifier for Clients {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(&self, end_entity: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: UnixTime) -> Result<ClientCertVerified, rustls::Error> {
        match &self.paired {
            Some(paired) if !paired.lock().unwrap().contains(end_entity.as_ref()) => Err(rustls::Error::InvalidCertificate(CertificateError::UnknownIssuer)),
            _ => Ok(ClientCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

fn acceptor(server: &Credentials, paired: Option<Arc<Mutex<HashSet<Vec<u8>>>>>) -> TlsAcceptor {
    let provider = provider();
    let mut config = ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_client_cert_verifier(Arc::new(Clients { provider, paired }))
        .with_single_cert(vec![server.cert_der().unwrap()], server.key_der().unwrap())
        .unwrap();
    config.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
    config.send_tls13_tickets = 0;
    TlsAcceptor::from(Arc::new(config))
}
