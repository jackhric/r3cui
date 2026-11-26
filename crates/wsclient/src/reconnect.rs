use crate::backoff::Backoff;
use crate::connection::Connection;
use std::fmt::Display;
use std::future::Future;
use std::time::{Duration, Instant};

/// What a driver does at each step of staying connected; `reconnect` runs the loop and the waiting between steps.
pub trait Session {
    type Error: Display;

    /// An attempt is about to start (numbered from 1; a long-lived connection sends it back to 1). Return false to
    /// stop reconnecting for good.
    fn connecting(&mut self, attempt: u32) -> bool;

    fn connect(&mut self) -> impl Future<Output = Result<Connection, Self::Error>> + Send;

    /// One connection's life: authenticate, subscribe, run. Returns why it ended.
    fn serve(&mut self, connection: Connection) -> impl Future<Output = Self::Error> + Send;

    /// How long to wait before the next attempt, given the backoff's suggestion. Override for errors that retrying
    /// sooner cannot fix.
    fn retry_in(&self, _error: &Self::Error, backoff: Duration) -> Duration {
        backoff
    }

    /// The attempt failed or the connection ended with `error`, and the next attempt comes in `retry_in`.
    fn lost(&mut self, error: Self::Error, retry_in: Duration);
}

/// Connects, serves, and connects again after a wait, until `connecting` says stop. Never returns otherwise.
pub async fn reconnect<S: Session>(session: &mut S) {
    let mut backoff = Backoff::new();
    loop {
        if !session.connecting(backoff.begin_attempt()) {
            return;
        }
        let error = match session.connect().await {
            Ok(connection) => {
                let up = Instant::now();
                let error = session.serve(connection).await;
                backoff.connection_ended(up.elapsed());
                error
            }
            Err(e) => e,
        };
        let retry_in = session.retry_in(&error, backoff.delay());
        session.lost(error, retry_in);
        tokio::time::sleep(retry_in).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeServer, Peer, Reply};
    use std::sync::Arc;

    struct Echo;

    impl Peer<()> for Echo {
        fn frame(&mut self, text: &str) -> Reply {
            Reply::say(text)
        }
    }

    #[derive(Default)]
    struct Recorder {
        url: String,
        attempts: Vec<u32>,
        lost: Vec<(String, Duration)>,
        stop_after: u32,
    }

    impl Session for Recorder {
        type Error = String;

        fn connecting(&mut self, attempt: u32) -> bool {
            self.attempts.push(attempt);
            self.attempts.len() as u32 <= self.stop_after
        }

        async fn connect(&mut self) -> Result<Connection, String> {
            crate::connect(self.url.as_str()).await.map_err(|e| e.to_string())
        }

        async fn serve(&mut self, mut connection: Connection) -> String {
            connection.out.send("hello".into()).unwrap();
            let echoed = connection.incoming.recv().await;
            assert_eq!(echoed.as_deref(), Some("hello"));
            match connection.incoming.recv().await {
                Some(_) => "unexpected frame".into(),
                None => "server closed".into(),
            }
        }

        fn lost(&mut self, error: String, retry_in: Duration) {
            self.lost.push((error, retry_in));
        }
    }

    #[tokio::test]
    async fn reconnects_after_the_server_drops_the_connection() {
        let server = Arc::new(FakeServer::<()>::start("127.0.0.1:0", || Echo).await);
        let kicker = {
            let server = server.clone();
            tokio::spawn(async move {
                for round in 1..=2 {
                    while server.accepted() < round {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    server.kick();
                }
            })
        };
        let mut session = Recorder { url: format!("ws://{}", server.addr()), stop_after: 2, ..Recorder::default() };
        reconnect(&mut session).await;
        kicker.await.unwrap();
        assert_eq!(session.attempts, [1, 2, 3]);
        assert_eq!(session.lost, [("server closed".to_string(), Duration::from_millis(500)), ("server closed".to_string(), Duration::from_secs(1))]);
        assert_eq!(server.accepted(), 2);
    }

    #[tokio::test]
    async fn an_unreachable_server_backs_off() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        drop(listener);
        let mut session = Recorder { url, stop_after: 2, ..Recorder::default() };
        reconnect(&mut session).await;
        let waits: Vec<_> = session.lost.iter().map(|(_, w)| *w).collect();
        assert_eq!(waits, [Duration::from_millis(500), Duration::from_secs(1)]);
    }
}
