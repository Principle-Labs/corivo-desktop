//! Telemetry capability — currently Sentry message events.
//!
//! Closed impl emits `sentry::capture_message` so the operator dashboard
//! sees `billing_dialog.opened` / `billing_checkout.started` / …
//! Open-source noop is a no-op; the Sentry SDK itself is also not
//! initialised in OSS builds (the closed build wires it in `lib.rs::run`
//! conditionally).

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde_json::Value;

use crate::error::Result;

#[async_trait]
pub trait TelemetryService: Send + Sync {
    /// Whether telemetry is wired (Sentry initialised). Surfaces through
    /// `Capabilities.telemetry` for the frontend's "我们收集了什么" UI;
    /// the OSS build shows "no telemetry" copy when this is false.
    fn is_available(&self) -> bool {
        false
    }

    /// Capture a business event. Fire-and-forget on the caller side:
    /// the trait's `Result` is here for future expansion (e.g. a
    /// privacy-respecting analytics backend that wants to surface
    /// rate-limit / batching errors), but the corivo Sentry impl will
    /// always return `Ok(())`.
    async fn track_event(
        &self,
        name: String,
        properties: Option<BTreeMap<String, Value>>,
    ) -> Result<()>;
}
