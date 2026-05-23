//! Phase 3 — `foreground.*` typed wrappers around the helper's frontmost-app
//! observer. Subscribed events arrive on the shared event bus as
//! [`super::events::HelperEvent::ForegroundAppActivated`].

use serde::Deserialize;
use serde_json::json;

use super::client::{CaptureClient, DEFAULT_REQUEST_DEADLINE};
use super::error::{CaptureError, Result};
use super::protocol::methods;

#[derive(Debug, Clone, Deserialize)]
pub struct ForegroundApp {
    #[serde(default)]
    pub pid: Option<i32>,
    #[serde(default)]
    pub bundle_id: Option<String>,
    #[serde(default)]
    pub app_name: Option<String>,
    #[serde(default)]
    pub window_title: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct AckResult {
    #[allow(dead_code)]
    #[serde(default)]
    ack: bool,
    #[allow(dead_code)]
    #[serde(default)]
    subscribed: bool,
}

impl CaptureClient {
    /// Tell the helper to start emitting `foreground.app_activated`
    /// events. Subscribers obtain them via [`Self::subscribe_events`].
    pub async fn subscribe_foreground(&self) -> Result<()> {
        if !self.capabilities().foreground_monitor {
            return Err(CaptureError::Unsupported(
                "helper does not support foreground.subscribe".into(),
            ));
        }
        let _: AckResult = self
            .request(
                methods::FOREGROUND_SUBSCRIBE,
                Some(json!({})),
                DEFAULT_REQUEST_DEADLINE,
            )
            .await?;
        Ok(())
    }

    pub async fn unsubscribe_foreground(&self) -> Result<()> {
        if !self.capabilities().foreground_monitor {
            return Err(CaptureError::Unsupported(
                "helper does not support foreground.unsubscribe".into(),
            ));
        }
        let _: AckResult = self
            .request(
                methods::FOREGROUND_UNSUBSCRIBE,
                Some(json!({})),
                DEFAULT_REQUEST_DEADLINE,
            )
            .await?;
        Ok(())
    }

    /// Synchronous read of the currently-frontmost app.
    pub async fn current_foreground(&self) -> Result<ForegroundApp> {
        if !self.capabilities().foreground_monitor {
            return Err(CaptureError::Unsupported(
                "helper does not support foreground.current".into(),
            ));
        }
        self.request(
            methods::FOREGROUND_CURRENT,
            Some(json!({})),
            DEFAULT_REQUEST_DEADLINE,
        )
        .await
    }
}
