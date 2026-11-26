//! Calls over one connection: each request gets an id, goes out in call order, and its reply (whenever it comes,
//! among events) completes the call that made it.

use crate::error::{Error, Result};
use crate::protocol::{field, MessageType};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use wsclient::{Pending, Unanswered};

type Outcome = Result<Value>;

#[derive(Clone)]
pub(crate) struct Client {
    out: wsclient::Outbox,
    pending: Pending<u32, Outcome>,
    next: Arc<AtomicU32>,
}

/// A request already sent, whose reply is yet to come.
pub(crate) struct Reply {
    id: u32,
    rx: oneshot::Receiver<Outcome>,
    pending: Pending<u32, Outcome>,
}

impl Client {
    pub(crate) fn new(out: wsclient::Outbox) -> Self {
        Self { out, pending: Pending::new(), next: Arc::new(AtomicU32::new(1)) }
    }

    /// Sends at once, so requests go out in the order they were made even if their replies are awaited elsewhere.
    pub(crate) fn request(&self, kind: MessageType, fields: Value) -> (u32, Reply) {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut body = json!({ field::ID: id, field::TYPE: kind.as_str() });
        if let (Some(body), Value::Object(fields)) = (body.as_object_mut(), fields) {
            body.extend(fields);
        }
        let rx = self.pending.insert(id);
        if self.out.send(body.to_string()).is_err() {
            self.pending.cancel(&id);
        }
        (id, Reply { id, rx, pending: self.pending.clone() })
    }

    pub(crate) async fn call(&self, kind: MessageType, fields: Value, timeout: Duration) -> Result<Value> {
        self.request(kind, fields).1.wait(timeout).await
    }

    /// A reply arrived.
    pub(crate) fn deliver(&self, id: u32, outcome: Outcome) {
        self.pending.deliver(&id, outcome);
    }
}

impl Reply {
    pub(crate) async fn wait(self, timeout: Duration) -> Result<Value> {
        match self.pending.wait(&self.id, self.rx, timeout).await {
            Ok(outcome) => outcome,
            Err(Unanswered::Closed) => Err(Error::Closed),
            Err(Unanswered::TimedOut) => Err(Error::Timeout),
        }
    }
}
