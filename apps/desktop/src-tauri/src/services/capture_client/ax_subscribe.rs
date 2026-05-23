//! Phase 3 — `ax.subscribe` / `ax.unsubscribe` typed wrappers. Subscribed
//! events arrive on the shared event bus as `HelperEvent::Ax*`.
//!
//! v1 helpers maintain at most one AX subscription at a time; calling
//! `subscribe` for a new pid implicitly retargets.

use serde::Deserialize;
use serde_json::json;

use super::client::{CaptureClient, DEFAULT_REQUEST_DEADLINE};
use super::error::{CaptureError, Result};
use super::protocol::methods;

/// Notification kinds the helper can subscribe to. Strings match the
/// wire enum on both platforms (Swift / C++).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxNotification {
    FocusedWindow,
    Title,
}

impl AxNotification {
    fn as_wire(&self) -> &'static str {
        match self {
            AxNotification::FocusedWindow => "focused_window",
            AxNotification::Title => "title",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct AckResult {
    #[allow(dead_code)]
    #[serde(default)]
    ack: bool,
    #[allow(dead_code)]
    #[serde(default)]
    subscribed: Option<Vec<String>>,
}

impl CaptureClient {
    pub async fn subscribe_ax(&self, pid: i32, notifications: &[AxNotification]) -> Result<()> {
        if !self.capabilities().ax_events {
            return Err(CaptureError::Unsupported(
                "helper does not support ax.subscribe".into(),
            ));
        }
        let names: Vec<&'static str> = notifications.iter().map(|n| n.as_wire()).collect();
        let _: AckResult = self
            .request(
                methods::AX_SUBSCRIBE,
                Some(json!({ "pid": pid, "notifications": names })),
                DEFAULT_REQUEST_DEADLINE,
            )
            .await?;
        Ok(())
    }

    pub async fn unsubscribe_ax(&self, pid: i32) -> Result<()> {
        if !self.capabilities().ax_events {
            return Err(CaptureError::Unsupported(
                "helper does not support ax.unsubscribe".into(),
            ));
        }
        let _: AckResult = self
            .request(
                methods::AX_UNSUBSCRIBE,
                Some(json!({ "pid": pid })),
                DEFAULT_REQUEST_DEADLINE,
            )
            .await?;
        Ok(())
    }
}
