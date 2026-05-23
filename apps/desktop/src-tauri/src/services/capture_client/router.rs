//! In-flight RPC router.
//!
//! Maps `Request.id` (a UUID assigned at call time) to a `oneshot::Sender`
//! so when the matching `Response` arrives off the read loop it gets handed
//! back to the awaiting task. Thread-safe; cheap to clone.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::oneshot;
use uuid::Uuid;

use super::error::{CaptureError, Result};
use super::protocol::Response;

/// Routes responses back to the task that submitted the request.
#[derive(Clone, Default)]
pub struct RpcRouter {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    in_flight: HashMap<Uuid, oneshot::Sender<Response>>,
    /// Set once the helper has crashed / shut down. Any new register call
    /// fails immediately rather than parking forever.
    closed: bool,
}

impl RpcRouter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a request; returns the receiver the caller awaits on.
    pub fn register(&self, id: Uuid) -> Result<oneshot::Receiver<Response>> {
        let (tx, rx) = oneshot::channel();
        let mut inner = self.lock();
        if inner.closed {
            return Err(CaptureError::HelperUnavailable);
        }
        if inner.in_flight.insert(id, tx).is_some() {
            return Err(CaptureError::Internal(format!(
                "duplicate request id: {id}"
            )));
        }
        Ok(rx)
    }

    /// Cancel a pending request without delivering a response. Used when the
    /// caller hits its deadline before the helper replies.
    pub fn cancel(&self, id: &Uuid) {
        let _ = self.lock().in_flight.remove(id);
    }

    /// Dispatch a response off the read loop. Drops the response silently if
    /// the matching request was already canceled (timeout) — that's not an
    /// error from the router's perspective.
    pub fn dispatch(&self, response: Response) {
        let id = response.id;
        let entry = self.lock().in_flight.remove(&id);
        if let Some(sender) = entry {
            // If the receiver was dropped (caller bailed) the send fails;
            // that's fine, we're done with this id.
            let _ = sender.send(response);
        } else {
            tracing::warn!(
                target: "capture_client",
                response_id = %id,
                "router.unmatched_response"
            );
        }
    }

    /// Mark closed and fail every still-pending request with
    /// [`CaptureError::HelperCrashed`]. Called when the read loop terminates
    /// (helper exited or stdout EOF).
    pub fn close_with_crash(&self) {
        let mut inner = self.lock();
        inner.closed = true;
        let pending: Vec<_> = inner.in_flight.drain().collect();
        drop(inner);
        for (_, sender) in pending {
            // We drop the sender → the awaiting `oneshot::Receiver` resolves
            // with `Err(RecvError)` which the client converts to
            // `HelperCrashed`. No need to explicitly send anything.
            drop(sender);
        }
    }

    pub fn in_flight_count(&self) -> usize {
        self.lock().in_flight.len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("router mutex poisoned")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fake_response(id: Uuid) -> Response {
        Response {
            id,
            ok: true,
            result: Some(json!({"pong": true})),
            error: None,
        }
    }

    #[tokio::test]
    async fn dispatch_delivers_response_to_register() {
        let router = RpcRouter::new();
        let id = Uuid::new_v4();
        let rx = router.register(id).unwrap();
        router.dispatch(fake_response(id));
        let resp = rx.await.unwrap();
        assert_eq!(resp.id, id);
        assert!(resp.ok);
        assert_eq!(router.in_flight_count(), 0);
    }

    #[tokio::test]
    async fn duplicate_register_id_errors() {
        let router = RpcRouter::new();
        let id = Uuid::new_v4();
        let _rx = router.register(id).unwrap();
        match router.register(id) {
            Err(CaptureError::Internal(msg)) => assert!(msg.contains("duplicate")),
            other => panic!("expected Internal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cancel_removes_from_in_flight() {
        let router = RpcRouter::new();
        let id = Uuid::new_v4();
        let _rx = router.register(id).unwrap();
        assert_eq!(router.in_flight_count(), 1);
        router.cancel(&id);
        assert_eq!(router.in_flight_count(), 0);
    }

    #[tokio::test]
    async fn close_with_crash_fails_pending_with_recv_error() {
        let router = RpcRouter::new();
        let id = Uuid::new_v4();
        let rx = router.register(id).unwrap();
        router.close_with_crash();
        // sender dropped → recv resolves to Err
        assert!(rx.await.is_err());
    }

    #[tokio::test]
    async fn register_after_close_errors() {
        let router = RpcRouter::new();
        router.close_with_crash();
        match router.register(Uuid::new_v4()) {
            Err(CaptureError::HelperUnavailable) => {}
            other => panic!("expected HelperUnavailable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatch_with_unknown_id_is_silent() {
        let router = RpcRouter::new();
        // Unknown id; should not panic. (Only logs a warn.)
        router.dispatch(fake_response(Uuid::new_v4()));
    }
}
