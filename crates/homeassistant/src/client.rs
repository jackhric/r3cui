//! Calls over one connection: each request gets an id, goes out in call order, and its reply (whenever it comes,
//! among events) completes the call that made it.

use crate::error::{Error, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

type Pending = Arc<Mutex<HashMap<u32, oneshot::Sender<Result<Value>>>>>;

#[derive(Clone)]
pub(crate) struct Client {
    out: mpsc::UnboundedSender<String>,
    pending: Pending,
    next: Arc<AtomicU32>,
}

/// A request already sent, whose reply is yet to come.
pub(crate) struct Reply {
    id: u32,
    rx: oneshot::Receiver<Result<Value>>,
    pending: Pending,
}

impl Client {
    pub(crate) fn new(out: mpsc::UnboundedSender<String>) -> Self {
        Self { out, pending: Arc::default(), next: Arc::new(AtomicU32::new(1)) }
    }

    /// Sends at once, so requests go out in the order they were made even if their replies are awaited elsewhere.
    pub(crate) fn request(&self, kind: &str, fields: Value) -> (u32, Reply) {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut body = json!({ "id": id, "type": kind });
        if let (Some(body), Value::Object(fields)) = (body.as_object_mut(), fields) {
            body.extend(fields);
        }
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        if self.out.send(body.to_string()).is_err() {
            self.pending.lock().unwrap().remove(&id);
        }
        (id, Reply { id, rx, pending: self.pending.clone() })
    }

    pub(crate) async fn call(&self, kind: &str, fields: Value, timeout: Duration) -> Result<Value> {
        self.request(kind, fields).1.wait(timeout).await
    }

    /// A reply arrived.
    pub(crate) fn deliver(&self, id: u32, outcome: Result<Value>) {
        if let Some(tx) = self.pending.lock().unwrap().remove(&id) {
            let _ = tx.send(outcome);
        }
    }
}

impl Reply {
    pub(crate) async fn wait(self, timeout: Duration) -> Result<Value> {
        match tokio::time::timeout(timeout, self.rx).await {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(_)) => Err(Error::Closed),
            Err(_) => {
                self.pending.lock().unwrap().remove(&self.id);
                Err(Error::Timeout)
            }
        }
    }
}
