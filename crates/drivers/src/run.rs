//! Small pieces every driver's tasks share.

use std::future::Future;
use std::time::Duration;
use tokio::sync::watch;

/// Keeps `view` as `f` of `source` until nobody watches the view or the source ends.
pub(crate) fn follow<S, V>(mut source: watch::Receiver<S>, view: watch::Sender<V>, f: impl Fn(&S) -> V + Send + 'static)
where
    S: Send + Sync + 'static,
    V: PartialEq + Send + Sync + 'static,
{
    tokio::spawn(async move {
        loop {
            let next = f(&source.borrow_and_update());
            view.send_if_modified(|v| {
                let changed = *v != next;
                *v = next;
                changed
            });
            tokio::select! {
                changed = source.changed() => if changed.is_err() { break },
                _ = view.closed() => break,
            }
        }
    });
}

/// The longest one command waits for the previous one's answer before going out anyway.
pub(crate) const PRESS_GAP: Duration = Duration::from_millis(40);

/// Runs a command in the order made: the next waits for this one's answer, but no longer than `PRESS_GAP`, so a
/// slow device never holds up the next press. Failures are logged.
pub(crate) async fn in_order<E: std::fmt::Display + Send + 'static>(what: String, call: impl Future<Output = Result<(), E>> + Send + 'static) {
    let mut call = Box::pin(call);
    let log = move |r: Result<(), E>| {
        if let Err(e) = r {
            tracing::warn!("{what}: {e}");
        }
    };
    match tokio::time::timeout(PRESS_GAP, &mut call).await {
        Ok(r) => log(r),
        Err(_) => {
            tokio::spawn(async move { log(call.await) });
        }
    }
}
