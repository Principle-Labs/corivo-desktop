//! Third-party connector framework.
//!
//! Closed impl wraps the existing `services::connector::ConnectorRegistry`
//! (Composio + per-vendor MCP launch). Open-source noop returns an
//! empty list so the Settings → Integrations tab is empty in the OSS
//! build (the tab itself is hidden via the capability flag).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::Result;

pub use crate::services::connector::ConnectorSummary;

/// Single Composio connection as the user sees it in Settings.
/// Status values mirror Composio's own enum verbatim — we don't map
/// them to a smaller set because the UI shows the literal status for
/// failed / expired / etc. (`ACTIVE` / `INITIATED` / `EXPIRED` / `FAILED`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "camelCase")]
pub struct ComposioConnection {
    pub connection_id: String,
    pub toolkit_slug: String,
    pub status: String,
    /// Human-readable account label (often the connected account's
    /// email or display name). `null` when Composio didn't surface
    /// one (some toolkits don't return user-level metadata).
    #[serde(default)]
    pub label: Option<String>,
    /// ISO-8601. `null` on legacy rows that pre-date Composio's
    /// createdAt field.
    #[serde(default)]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "camelCase")]
pub struct ComposioConnectionsResponse {
    pub connections: Vec<ComposioConnection>,
}

/// Returned by `composio_create_connection_link`. Frontend opens
/// `redirect_url` in the system browser; Composio's hosted page walks
/// the user through OAuth consent and writes the connection back to
/// Composio. Frontend then polls list/connections to confirm.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "camelCase")]
pub struct ComposioConnectionLink {
    pub connection_id: String,
    pub redirect_url: String,
    pub toolkit_slug: String,
}

#[async_trait]
pub trait ConnectorsService: Send + Sync {
    /// False in the OSS build — Settings → Integrations is hidden.
    fn is_available(&self) -> bool {
        false
    }

    async fn list(&self) -> Result<Vec<ConnectorSummary>>;

    async fn enable(&self, id: String) -> Result<ConnectorSummary>;

    async fn disable(&self, id: String) -> Result<()>;

    /// Long-running: opens the system browser and waits up to 5 min for
    /// the user to complete OAuth consent. Tauri command handler must
    /// surface a spinner.
    async fn connect(&self, id: String) -> Result<ConnectorSummary>;

    /// Multi-connector connect on a single provider — opens the browser
    /// at most once with the union of every listed connector's manifest
    /// scopes. Used by the Settings → Integrations `ProviderCard`.
    async fn provider_connect(
        &self,
        provider: String,
        connector_ids: Vec<String>,
    ) -> Result<Vec<ConnectorSummary>>;

    /// Long-running like `connect()`: spawns a sidecar bootstrap that
    /// drives the vendor MCP server's OAuth flow. Used by `mcpServer`-
    /// shape connectors (Linear, future Notion/GitHub MCP).
    async fn install_mcp(&self, id: String) -> Result<ConnectorSummary>;

    async fn disconnect(&self, id: String) -> Result<ConnectorSummary>;

    async fn composio_list_connections(&self) -> Result<ComposioConnectionsResponse>;

    async fn composio_create_connection_link(
        &self,
        toolkit_slug: String,
        redirect_url: Option<String>,
    ) -> Result<ComposioConnectionLink>;

    async fn composio_disconnect(&self, connection_id: String) -> Result<()>;

    /// Extra MCP server specs supplied by a managed cloud backend.
    /// OSS builds return an empty list; closed builds may inject
    /// server-side proxy transports such as Composio.
    fn hosted_mcp_specs(&self) -> Vec<serde_json::Value> {
        Vec::new()
    }
}
