use futures_util::stream::SplitStream;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

pub type Outbox = mpsc::UnboundedSender<Message>;

pub enum Frame {
    Text(String),
    Binary(Vec<u8>),
}

/// One accepted WebSocket: frames in through `next`, frames out through the `Outbox`. Pings are answered here.
pub struct Conn {
    pub out: Outbox,
    reader: SplitStream<WebSocketStream<TcpStream>>,
}

impl Conn {
    pub async fn accept(tcp: TcpStream) -> Option<Self> {
        let ws = tokio_tungstenite::accept_async(tcp).await.ok()?;
        let (mut sink, reader) = ws.split();
        let (out, mut queue) = mpsc::unbounded_channel::<Message>();
        tokio::spawn(async move {
            while let Some(message) = queue.recv().await {
                let closing = matches!(message, Message::Close(_));
                if sink.send(message).await.is_err() || closing {
                    break;
                }
            }
            let _ = sink.close().await;
        });
        Some(Self { out, reader })
    }

    pub async fn next(&mut self) -> Option<Frame> {
        loop {
            match self.reader.next().await? {
                Ok(Message::Text(t)) => return Some(Frame::Text(t)),
                Ok(Message::Binary(b)) => return Some(Frame::Binary(b)),
                Ok(Message::Ping(payload)) => {
                    let _ = self.out.send(Message::Pong(payload));
                }
                Ok(Message::Close(_)) | Err(_) => return None,
                Ok(_) => {}
            }
        }
    }
}
