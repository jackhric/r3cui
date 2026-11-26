//! A stream of varint-length-prefixed protobuf messages.

use crate::error::{Error, Result};
use prost::Message;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const MAX_MESSAGE: usize = 1 << 20;
const MAX_VARINT_BYTES: usize = 5;

pub struct Channel<S> {
    stream: S,
    buf: Vec<u8>,
}

impl<S: AsyncRead + AsyncWrite + Unpin> Channel<S> {
    pub fn new(stream: S) -> Self {
        Self { stream, buf: Vec::new() }
    }

    pub async fn send<M: Message>(&mut self, message: &M) -> Result<()> {
        self.stream.write_all(&message.encode_length_delimited_to_vec()).await?;
        self.stream.flush().await?;
        Ok(())
    }

    /// Cancel-safe: bytes read so far stay in the buffer, so it can sit in a `select!`.
    pub async fn recv<M: Message + Default>(&mut self) -> Result<M> {
        loop {
            if let Some((len, header)) = self.frame()? {
                let message = M::decode(&self.buf[header..header + len])?;
                self.buf.drain(..header + len);
                return Ok(message);
            }
            if self.stream.read_buf(&mut self.buf).await.map_err(Error::from_read)? == 0 {
                return Err(Error::Closed);
            }
        }
    }

    /// The length of the first complete message and of its prefix, once all of it has arrived.
    fn frame(&self) -> Result<Option<(usize, usize)>> {
        let mut len = 0usize;
        for (i, byte) in self.buf.iter().take(MAX_VARINT_BYTES).enumerate() {
            len |= usize::from(byte & 0x7f) << (7 * i);
            if byte & 0x80 == 0 {
                if len > MAX_MESSAGE {
                    return Err(Error::TooLarge(len));
                }
                return Ok((self.buf.len() >= i + 1 + len).then_some((len, i + 1)));
            }
        }
        if self.buf.len() >= MAX_VARINT_BYTES {
            return Err(Error::TooLarge(usize::MAX));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::remote::{PingRequest, RemoteMessage};
    use tokio::io::duplex;

    #[tokio::test]
    async fn messages_survive_being_split_and_glued() {
        let (a, mut b) = duplex(64);
        let mut ch = Channel::new(a);
        let one = RemoteMessage { ping_request: Some(PingRequest { val1: 7, val2: 0 }), ..Default::default() };
        let two = RemoteMessage::launch("https://example.test/app");
        let mut bytes = one.encode_length_delimited_to_vec();
        bytes.extend(two.encode_length_delimited_to_vec());
        tokio::spawn(async move {
            for chunk in bytes.chunks(3) {
                b.write_all(chunk).await.unwrap();
                tokio::task::yield_now().await;
            }
        });
        assert_eq!(ch.recv::<RemoteMessage>().await.unwrap(), one);
        assert_eq!(ch.recv::<RemoteMessage>().await.unwrap(), two);
        assert!(matches!(ch.recv::<RemoteMessage>().await, Err(Error::Closed)));
    }

    #[tokio::test]
    async fn an_oversized_prefix_is_refused() {
        let (a, mut b) = duplex(64);
        b.write_all(&[0xff, 0xff, 0xff, 0x7f]).await.unwrap();
        let mut ch = Channel::new(a);
        assert!(matches!(ch.recv::<RemoteMessage>().await, Err(Error::TooLarge(_))));
    }
}
