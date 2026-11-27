//! `FakeBridge`: the bridge's side of the link, for r3cui's tests with no device. It listens on an ephemeral port;
//! a test scripts a session and reads what r3cui sent back.

use crate::link::{BridgeMsg, SampleFormat, SessionId, UiMsg};
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

const WAIT: Duration = Duration::from_secs(5);

type Outbox = Arc<Mutex<Option<mpsc::UnboundedSender<Message>>>>;

pub struct FakeBridge {
    addr: SocketAddr,
    current: Outbox,
    connections: watch::Receiver<usize>,
    received: mpsc::UnboundedReceiver<UiMsg>,
    task: JoinHandle<()>,
}

impl FakeBridge {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let current = Outbox::default();
        let (count_tx, connections) = watch::channel(0);
        let (ui_tx, received) = mpsc::unbounded_channel();
        let slot = current.clone();
        let task = tokio::spawn(async move {
            let mut count = 0;
            while let Ok((tcp, _)) = listener.accept().await {
                let Ok(ws) = tokio_tungstenite::accept_async(tcp).await else {
                    continue;
                };
                let (mut sink, mut stream) = ws.split();
                let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();
                *slot.lock().unwrap() = Some(out_tx);
                count += 1;
                let _ = count_tx.send(count);
                let ui_tx = ui_tx.clone();
                tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            out = out_rx.recv() => match out {
                                Some(m) => {
                                    if sink.send(m).await.is_err() {
                                        break;
                                    }
                                }
                                None => {
                                    let _ = sink.close().await;
                                    break;
                                }
                            },
                            frame = stream.next() => match frame {
                                Some(Ok(Message::Text(t))) => {
                                    if let Ok(m) = serde_json::from_str(&t) {
                                        let _ = ui_tx.send(m);
                                    }
                                }
                                Some(Ok(_)) => {}
                                _ => break,
                            },
                        }
                    }
                });
            }
        });
        Self {
            addr,
            current,
            connections,
            received,
            task,
        }
    }

    /// `host:port`, for `connect_to`.
    pub fn addr(&self) -> String {
        self.addr.to_string()
    }

    pub async fn wait_for_client(&mut self) {
        let wait = self.connections.wait_for(|n| *n > 0);
        tokio::time::timeout(WAIT, wait)
            .await
            .expect("no client connected")
            .expect("fake bridge stopped");
    }

    fn push(&self, frame: Message) {
        let current = self.current.lock().unwrap();
        current
            .as_ref()
            .expect("no client connected")
            .send(frame)
            .expect("client connection is gone");
    }

    pub fn send(&self, msg: &BridgeMsg) {
        self.push(Message::Text(serde_json::to_string(msg).unwrap()));
    }

    pub fn send_audio(&self, pcm: impl Into<Vec<u8>>) {
        self.push(Message::Binary(pcm.into()));
    }

    /// Begin, the chunks, End: a whole session of 16 kHz mono `i16`.
    pub fn script_session(&self, session_id: SessionId, chunks: &[&[u8]]) {
        self.send(&BridgeMsg::Begin {
            session_id,
            sample_rate: 16000,
            channels: 1,
            format: SampleFormat::I16,
        });
        for chunk in chunks {
            self.send_audio(*chunk);
        }
        self.send(&BridgeMsg::End { session_id });
    }

    /// What r3cui sent next; panics after a few seconds of silence.
    pub async fn next_ui_msg(&mut self) -> UiMsg {
        tokio::time::timeout(WAIT, self.received.recv())
            .await
            .expect("r3cui sent nothing")
            .expect("fake bridge stopped")
    }

    /// Hangs up on the connected client; it should reconnect.
    pub fn drop_client(&self) {
        self.current.lock().unwrap().take();
    }
}

impl Drop for FakeBridge {
    fn drop(&mut self) {
        self.task.abort();
    }
}
