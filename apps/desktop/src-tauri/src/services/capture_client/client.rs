//! Public capture-client API.
//!
//! Phase 0 surface:
//! - [`CaptureClient::spawn`] — spawn helper, complete hello handshake
//! - [`CaptureClient::ping`]  — liveness sanity check
//! - [`CaptureClient::shutdown`] — graceful stop
//! - capability / platform / health accessors
//!
//! Future phases extend this with `start_recording`, `capture_screen`,
//! `ax_query`, `subscribe_foreground`, etc. — each gated by a capability
//! check via [`CaptureClient::capabilities`].

use std::time::Duration;

use serde::de::DeserializeOwned;
use tokio::time::timeout;
use uuid::Uuid;

use super::error::{CaptureError, Result};
use super::events::{EventBus, HelperEvent};
use super::process::{HelperProcess, SpawnOptions};
use super::protocol::{methods, Capabilities, Message, PlatformInfo, ProtocolVersion, Request};

pub use super::health::HelperHealthStatus as HelperHealth;

/// Default per-call deadline. Individual methods (e.g. `ax_query` with a
/// long max-walk window) may override.
pub const DEFAULT_REQUEST_DEADLINE: Duration = Duration::from_secs(5);

/// How long [`CaptureClient::shutdown`] waits for the helper to exit
/// after the shutdown response is acknowledged. Spec § 6.4: 5 + 2 seconds.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(7);

/// Snapshot of the helper's hello — read-only, safe to expose.
#[derive(Debug, Clone)]
pub struct HelperSnapshot {
    pub capabilities: Capabilities,
    pub platform: PlatformInfo,
    pub helper_version: String,
    pub protocol: ProtocolVersion,
}

pub struct CaptureClient {
    process: HelperProcess,
    snapshot: HelperSnapshot,
}

impl CaptureClient {
    /// Spawn the helper at `binary_path` and complete the hello handshake.
    /// Returns a ready-to-use client whose [`Self::capabilities`] reflects
    /// what the helper advertised.
    pub async fn spawn(opts: SpawnOptions) -> Result<Self> {
        let client_version = opts.client_version.clone();
        let (process, hello) = HelperProcess::spawn(opts).await?;
        let snapshot = HelperSnapshot {
            capabilities: hello.capabilities,
            platform: hello.platform,
            helper_version: hello.helper_version,
            // Pick the same protocol the spawn flow negotiated. Phase 0
            // only has v1, so this is unconditional; later phases will
            // store the negotiated value on the process and query it here.
            protocol: ProtocolVersion::V1,
        };
        tracing::info!(
            target: "capture_client",
            helper_version = %snapshot.helper_version,
            protocol = ?snapshot.protocol,
            os = ?snapshot.platform.os,
            os_version = %snapshot.platform.os_version,
            arch = ?snapshot.platform.arch,
            audio = snapshot.capabilities.audio_record,
            screen = snapshot.capabilities.screen_capture,
            ax = snapshot.capabilities.ax_query,
            ocr = snapshot.capabilities.ocr_local,
            client_version = %client_version,
            "capture_client.spawned"
        );
        Ok(CaptureClient { process, snapshot })
    }

    pub fn capabilities(&self) -> &Capabilities {
        &self.snapshot.capabilities
    }

    pub fn platform(&self) -> &PlatformInfo {
        &self.snapshot.platform
    }

    pub fn helper_version(&self) -> &str {
        &self.snapshot.helper_version
    }

    pub fn protocol_version(&self) -> ProtocolVersion {
        self.snapshot.protocol
    }

    pub fn health(&self) -> HelperHealth {
        self.process.health().status()
    }

    /// Subscribe to the broadcast event bus. Receivers see every event
    /// emitted from the time of subscription forward; lagging subscribers
    /// see `RecvError::Lagged` from the receiver but do not stall the read
    /// loop.
    pub fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<HelperEvent> {
        self.process.events().subscribe()
    }

    /// `ping` RPC. Useful for tests and as a tighter liveness check than
    /// the heartbeat-driven [`Self::health`] (which only marks "Crashed"
    /// after the threshold).
    pub async fn ping(&self) -> Result<()> {
        #[derive(serde::Deserialize)]
        struct PongResult {
            pong: bool,
        }
        let resp: PongResult = self
            .request(methods::PING, None, DEFAULT_REQUEST_DEADLINE)
            .await?;
        if !resp.pong {
            return Err(CaptureError::Protocol("ping: pong=false".into()));
        }
        Ok(())
    }

    /// Graceful shutdown. Sends `shutdown` RPC, awaits ack, then waits
    /// up to [`SHUTDOWN_GRACE`] for the helper to exit. If the helper
    /// hasn't exited by then it gets force-killed.
    ///
    /// Consumes the client; calling code must drop any remaining
    /// references first.
    pub async fn shutdown(mut self) -> Result<()> {
        #[derive(serde::Deserialize)]
        struct AckResult {
            ack: bool,
        }
        let result: Result<AckResult> = self
            .request(methods::SHUTDOWN, None, DEFAULT_REQUEST_DEADLINE)
            .await;
        match &result {
            Ok(ack) if !ack.ack => tracing::warn!(
                target: "capture_client",
                "shutdown.helper_acked_with_false"
            ),
            Ok(_) => {}
            Err(e) => tracing::warn!(
                target: "capture_client",
                error = %e,
                "shutdown.rpc_failed_proceeding_to_kill"
            ),
        }
        // Wait for child to exit naturally; force-kill if it overstays.
        match timeout(SHUTDOWN_GRACE, self.process.wait()).await {
            Ok(Ok(status)) => {
                tracing::info!(
                    target: "capture_client",
                    status = ?status,
                    "capture_client.shutdown_complete"
                );
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    target: "capture_client",
                    error = %e,
                    "capture_client.shutdown_wait_failed"
                );
            }
            Err(_) => {
                tracing::warn!(
                    target: "capture_client",
                    grace = ?SHUTDOWN_GRACE,
                    "capture_client.shutdown_grace_exceeded_force_kill"
                );
                self.process.force_kill();
                let _ = self.process.wait().await;
            }
        }
        Ok(())
    }

    /// Bottom-half of the public API — type-erased request/response over
    /// the IPC channel. Public RPC methods build on this.
    pub(crate) async fn request<T>(
        &self,
        method: &str,
        payload: Option<serde_json::Value>,
        deadline: Duration,
    ) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let id = Uuid::new_v4();
        let rx = self.process.router().register(id)?;
        let req = Message::Request(Request {
            id,
            method: method.to_string(),
            payload,
        });
        self.process.send(req).await?;
        let resp = match timeout(deadline, rx).await {
            Ok(Ok(r)) => r,
            // `oneshot::Receiver` resolves to Err when the Sender is
            // dropped, which only happens via `RpcRouter::close_with_crash`.
            Ok(Err(_)) => return Err(CaptureError::HelperCrashed),
            Err(_) => {
                self.process.router().cancel(&id);
                return Err(CaptureError::Timeout(deadline));
            }
        };
        if !resp.ok {
            let err = resp.error.ok_or_else(|| {
                CaptureError::Protocol("response.ok=false but no error field".into())
            })?;
            return Err(CaptureError::from_response_error(err));
        }
        let result_value = resp.result.unwrap_or(serde_json::Value::Null);
        serde_json::from_value(result_value)
            .map_err(|e| CaptureError::Protocol(format!("response decode: {e}")))
    }
}

/// Re-exported for the events bus type alias above.
#[allow(dead_code)]
fn _events_bus_type_check(b: &EventBus) -> tokio::sync::broadcast::Receiver<HelperEvent> {
    b.subscribe()
}
