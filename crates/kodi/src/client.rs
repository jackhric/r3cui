//! Calls over one connection: each request gets an id, goes out in call order, and its reply (whenever it comes,
//! among notifications) completes the call that made it.

use crate::error::{Error, Result};
use crate::ids::RequestId;
use crate::rpc::{decode, Method, Request, RpcError};
use serde_json::value::RawValue;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use wsclient::{Pending, Unanswered};

type Reply = std::result::Result<Box<RawValue>, RpcError>;

#[derive(Clone)]
pub(crate) struct Client {
    out: wsclient::Outbox,
    pending: Pending<RequestId, Reply>,
    next: Arc<AtomicU32>,
}

impl Client {
    pub(crate) fn new(out: wsclient::Outbox) -> Self {
        Self { out, pending: Pending::new(), next: Arc::new(AtomicU32::new(1)) }
    }

    pub(crate) async fn call<M: Method>(&self, method: &M, timeout: Duration) -> Result<M::Output> {
        let id = RequestId(self.next.fetch_add(1, Ordering::Relaxed));
        let request = Request::new(id, method);
        let rx = self.pending.insert(id);
        if self.out.send(request.body).is_err() {
            self.pending.cancel(&id);
            return Err(Error::Closed);
        }
        match self.pending.wait(&id, rx, timeout).await {
            Ok(reply) => decode::<M>(reply),
            Err(Unanswered::Closed) => Err(Error::Closed),
            Err(Unanswered::TimedOut) => Err(Error::Timeout { method: M::NAME, after: timeout }),
        }
    }

    /// A reply arrived.
    pub(crate) fn deliver(&self, id: RequestId, reply: Reply) {
        self.pending.deliver(&id, reply);
    }
}
