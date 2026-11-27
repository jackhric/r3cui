//! Calls over one connection: each request gets an id, goes out in call order, and its reply (whenever it comes,
//! among events) completes the call that made it.

use crate::error::{Error, Result};
use crate::protocol::{field, MessageType};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::mpsc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use r3cui_sdk::ws::{Pending, Unanswered};

type Outcome = Result<Value>;

#[derive(Clone)]
pub(crate) struct Client {
    out: r3cui_sdk::ws::Outbox,
    pending: Pending<u32, Outcome>,
    next: Arc<AtomicU32>,
    routes: Arc<Mutex<HashMap<u32, mpsc::UnboundedSender<Value>>>>,
    /// The voice run in flight; a new one cancels it.
    pub(crate) voice_run: Arc<Mutex<Option<u32>>>,
}

/// A request already sent, whose reply is yet to come.
pub(crate) struct Reply {
    id: u32,
    rx: oneshot::Receiver<Outcome>,
    pending: Pending<u32, Outcome>,
}

impl Client {
    pub(crate) fn new(out: r3cui_sdk::ws::Outbox) -> Self {
        Self { out, pending: Pending::new(), next: Arc::new(AtomicU32::new(1)), routes: Arc::default(), voice_run: Arc::default() }
    }

    pub(crate) fn send_binary(&self, frame: Vec<u8>) -> Result<()> {
        self.out.send_binary(frame).map_err(|_| Error::Closed)
    }

    /// Sends at once, so requests go out in the order they were made even if their replies are awaited elsewhere.
    pub(crate) fn request(&self, kind: MessageType, fields: Value) -> (u32, Reply) {
        self.send(kind, fields, None)
    }

    /// A request whose id also carries events back, to the returned receiver. The route exists before the request
    /// goes out, so no event can arrive ahead of it, and events and the reply reach the caller in the order they came.
    pub(crate) fn subscribe(&self, kind: MessageType, fields: Value) -> (u32, Reply, mpsc::UnboundedReceiver<Value>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let (id, reply) = self.send(kind, fields, Some(tx));
        (id, reply, rx)
    }

    fn send(&self, kind: MessageType, fields: Value, route: Option<mpsc::UnboundedSender<Value>>) -> (u32, Reply) {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut body = json!({ field::ID: id, field::TYPE: kind.as_str() });
        if let (Some(body), Value::Object(fields)) = (body.as_object_mut(), fields) {
            body.extend(fields);
        }
        let rx = self.pending.insert(id);
        if let Some(route) = route {
            self.routes.lock().unwrap().insert(id, route);
        }
        if self.out.send(body.to_string()).is_err() {
            self.pending.cancel(&id);
            self.routes.lock().unwrap().remove(&id);
        }
        (id, Reply { id, rx, pending: self.pending.clone() })
    }

    pub(crate) async fn call(&self, kind: MessageType, fields: Value, timeout: Duration) -> Result<Value> {
        self.request(kind, fields).1.wait(timeout).await
    }

    /// An event arrived for a request made with `subscribe`; false when nothing follows that id.
    pub(crate) fn route(&self, id: u32, event: Value) -> bool {
        self.routes.lock().unwrap().get(&id).is_some_and(|route| route.send(event).is_ok())
    }

    /// Stops following `id`; true if it still was, which is when Home Assistant has to be told.
    pub(crate) fn release(&self, id: u32) -> bool {
        self.routes.lock().unwrap().remove(&id).is_some()
    }

    /// Ends every routed stream: the connection is gone.
    pub(crate) fn close_routes(&self) {
        self.routes.lock().unwrap().clear();
    }

    /// Releases `id`, and asks Home Assistant to stop it if it was still running.
    pub(crate) fn cancel(&self, id: u32) {
        if self.release(id) {
            let _ = self.request(MessageType::UnsubscribeEvents, json!({ field::SUBSCRIPTION: id }));
        }
    }

    /// A voice run is over, one way or another: cancel it if it is still going, and stop counting it as current.
    pub(crate) fn end_voice_run(&self, id: u32) {
        self.cancel(id);
        let mut current = self.voice_run.lock().unwrap();
        if *current == Some(id) {
            *current = None;
        }
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
