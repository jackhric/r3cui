//! For testing a driver against a fake device: `FakeServer` takes the listening, accepting, counting and dropping
//! of connections off the fake's hands, leaving it to say what a device says (a `Peer` per connection); `until` and
//! `eventually` wait for the driver to get there.

use futures_util::{SinkExt, StreamExt};
use std::fmt::Debug;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;

/// How long `until` and `eventually` wait before they fail the test.
pub const WAIT: Duration = Duration::from_secs(5);

const CHECK_EVERY: Duration = Duration::from_millis(20);
const BROADCAST_BACKLOG: usize = 256;

/// What a fake sends in answer to something, in order, and whether it then hangs up.
#[derive(Debug, Default)]
pub struct Reply {
    pub frames: Vec<String>,
    pub close: bool,
}

impl Reply {
    pub fn nothing() -> Self {
        Self::default()
    }

    pub fn say(frame: impl Into<String>) -> Self {
        Self { frames: vec![frame.into()], close: false }
    }

    pub fn say_all(frames: Vec<String>) -> Self {
        Self { frames, close: false }
    }

    pub fn then_close(mut self) -> Self {
        self.close = true;
        self
    }
}

/// One connection's side of the conversation. `M` is what the test broadcasts to every connection.
pub trait Peer<M>: Send + 'static {
    /// The connection is up; some devices speak first.
    fn opened(&mut self) -> Reply {
        Reply::nothing()
    }

    fn frame(&mut self, text: &str) -> Reply;

    fn broadcast(&mut self, _message: &M) -> Reply {
        Reply::nothing()
    }
}

pub struct FakeServer<M> {
    addr: SocketAddr,
    accepted: Arc<AtomicUsize>,
    open: Arc<AtomicUsize>,
    kicks: watch::Sender<u64>,
    broadcasts: broadcast::Sender<M>,
    accepting: JoinHandle<()>,
}

struct Open(Arc<AtomicUsize>);

impl Drop for Open {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl<M: Clone + Send + 'static> FakeServer<M> {
    /// Listens on `addr` (`"127.0.0.1:0"` for any free port); each connection gets a peer from `make_peer`.
    pub async fn start<P: Peer<M>>(addr: &str, make_peer: impl Fn() -> P + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind(addr).await.expect("bind");
        let addr = listener.local_addr().expect("bound");
        let accepted = Arc::new(AtomicUsize::new(0));
        let open = Arc::new(AtomicUsize::new(0));
        let kicks = watch::channel(0).0;
        let broadcasts = broadcast::channel(BROADCAST_BACKLOG).0;
        let accepting = tokio::spawn({
            let (accepted, open, kicks, broadcasts) = (accepted.clone(), open.clone(), kicks.clone(), broadcasts.clone());
            async move {
                while let Ok((stream, _)) = listener.accept().await {
                    accepted.fetch_add(1, Ordering::SeqCst);
                    open.fetch_add(1, Ordering::SeqCst);
                    tokio::spawn(serve(stream, make_peer(), Open(open.clone()), broadcasts.subscribe(), kicks.subscribe()));
                }
            }
        });
        Self { addr, accepted, open, kicks, broadcasts, accepting }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Connections ever accepted: tells a reconnect from the first connection.
    pub fn accepted(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }

    /// Connections open now.
    pub fn open(&self) -> usize {
        self.open.load(Ordering::SeqCst)
    }

    /// Drops every connection; the listener stays up, so a client can reconnect.
    pub fn kick(&self) {
        self.kicks.send_modify(|generation| *generation += 1);
    }

    /// Offers `message` to every open connection's peer.
    pub fn broadcast(&self, message: M) {
        let _ = self.broadcasts.send(message);
    }
}

impl<M> Drop for FakeServer<M> {
    fn drop(&mut self) {
        self.accepting.abort();
        self.kicks.send_modify(|generation| *generation += 1);
    }
}

async fn serve<M: Clone + Send + 'static>(stream: TcpStream, mut peer: impl Peer<M>, _open: Open, mut pushes: broadcast::Receiver<M>, mut kicked: watch::Receiver<u64>) {
    let Ok(socket) = tokio_tungstenite::accept_async(stream).await else { return };
    let (mut sink, mut source) = socket.split();
    let mut reply = peer.opened();
    loop {
        let Reply { frames, close } = reply;
        for frame in frames {
            if sink.send(Message::Text(frame)).await.is_err() {
                return;
            }
        }
        if close {
            break;
        }
        reply = tokio::select! {
            frame = source.next() => match frame {
                Some(Ok(Message::Text(text))) => peer.frame(&text),
                Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                Some(Ok(_)) => Reply::nothing(),
            },
            push = pushes.recv() => match push {
                Ok(message) => peer.broadcast(&message),
                Err(broadcast::error::RecvError::Lagged(_)) => Reply::nothing(),
                Err(broadcast::error::RecvError::Closed) => break,
            },
            _ = kicked.changed() => break,
        };
    }
    let _ = sink.close().await;
}

/// Waits (up to `WAIT`) until the watched value satisfies `ok`, and returns it. Panics saying `what` it never saw.
pub async fn until<T: Clone + Debug>(rx: &mut watch::Receiver<T>, what: &str, ok: impl Fn(&T) -> bool) -> T {
    let found = tokio::time::timeout(WAIT, rx.wait_for(|value| ok(value))).await.ok().and_then(Result::ok).map(|value| value.clone());
    found.unwrap_or_else(|| panic!("never saw {what}; last value {:?}", rx.borrow()))
}

/// Checks every so often (up to `WAIT`) until `check` is true. Panics saying `what` it never saw.
pub async fn eventually<F: Future<Output = bool>>(what: &str, mut check: impl FnMut() -> F) {
    let start = Instant::now();
    while !check().await {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        tokio::time::sleep(CHECK_EVERY).await;
    }
}
