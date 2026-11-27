use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

/// Why a call got no reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Unanswered {
    #[error("connection closed")]
    Closed,
    #[error("no reply in time")]
    TimedOut,
}

/// Calls sent and not yet answered, by id. Clones share the table.
pub struct Pending<Id, Reply> {
    calls: Arc<Mutex<HashMap<Id, oneshot::Sender<Reply>>>>,
}

impl<Id, Reply> Clone for Pending<Id, Reply> {
    fn clone(&self) -> Self {
        Self { calls: self.calls.clone() }
    }
}

impl<Id, Reply> Default for Pending<Id, Reply> {
    fn default() -> Self {
        Self { calls: Arc::default() }
    }
}

impl<Id: Hash + Eq, Reply> Pending<Id, Reply> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a call before its frame is sent, so a fast reply cannot beat it.
    pub fn insert(&self, id: Id) -> oneshot::Receiver<Reply> {
        let (tx, rx) = oneshot::channel();
        self.calls.lock().unwrap().insert(id, tx);
        rx
    }

    /// Completes the call waiting under `id`; false when none is (it timed out, or the frame was not a reply).
    pub fn deliver(&self, id: &Id, reply: Reply) -> bool {
        match self.calls.lock().unwrap().remove(id) {
            Some(tx) => tx.send(reply).is_ok(),
            None => false,
        }
    }

    pub fn cancel(&self, id: &Id) {
        self.calls.lock().unwrap().remove(id);
    }

    /// Fails every waiting call as `Unanswered::Closed`: the connection is gone.
    pub fn clear(&self) {
        self.calls.lock().unwrap().clear();
    }

    pub fn len(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Waits for the reply to the call registered under `id`, and drops the call if none comes within `timeout`.
    pub async fn wait(&self, id: &Id, rx: oneshot::Receiver<Reply>, timeout: Duration) -> Result<Reply, Unanswered> {
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(reply)) => Ok(reply),
            Ok(Err(_)) => Err(Unanswered::Closed),
            Err(_) => {
                self.cancel(id);
                Err(Unanswered::TimedOut)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_reply_completes_its_call_only() {
        let pending = Pending::<u32, &str>::new();
        let (one, two) = (pending.insert(1), pending.insert(2));
        assert!(pending.deliver(&2, "second"));
        assert_eq!(pending.wait(&2, two, Duration::from_secs(1)).await, Ok("second"));
        assert_eq!(pending.len(), 1);
        assert!(pending.deliver(&1, "first"));
        assert_eq!(pending.wait(&1, one, Duration::from_secs(1)).await, Ok("first"));
        assert!(pending.is_empty());
        assert!(!pending.deliver(&1, "again"), "a reply is delivered once");
    }

    #[tokio::test(start_paused = true)]
    async fn a_timeout_removes_the_call() {
        let pending = Pending::<u32, ()>::new();
        let rx = pending.insert(7);
        assert_eq!(pending.wait(&7, rx, Duration::from_secs(5)).await, Err(Unanswered::TimedOut));
        assert!(pending.is_empty());
        assert!(!pending.deliver(&7, ()), "a late reply finds nobody");
    }

    #[tokio::test]
    async fn clearing_closes_the_waiting_calls() {
        let pending = Pending::<u32, ()>::new();
        let rx = pending.insert(1);
        let waiting = tokio::spawn({
            let pending = pending.clone();
            async move { pending.wait(&1, rx, Duration::from_secs(60)).await }
        });
        tokio::task::yield_now().await;
        pending.clear();
        assert_eq!(waiting.await.unwrap(), Err(Unanswered::Closed));
    }
}
