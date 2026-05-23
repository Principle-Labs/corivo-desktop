//! Agent-facing snapshot + stdio refresh broker.
//!
//! Two responsibilities:
//!
//! 1. **Spawn-time snapshot** (`ConnectorTokenSnapshot`): the shape
//!    that gets written into the agent sidecar's input JSON for every
//!    turn. Filled by `ConnectorRegistry::enabled_snapshot` —
//!    "enabled + connected + has-fresh-token" connectors only.
//!
//! 2. **Mid-turn refresh** (`handle_host_request`): the agent stdout
//!    NDJSON channel can emit `{ type: "host_request", op:
//!    "refresh_connector_token", connector_id: "gmail" }` if an
//!    access_token dies during a long turn. The runner-side handler
//!    calls back into `ConnectorRegistry::get_access_token` (which
//!    transparently refreshes) and writes a `host_response` back to
//!    the sidecar's stdin.
//!
//! The actual stdio wiring (NDJSON parser + writer) lives in
//! `services::exec_agent::runner` because that's where the sidecar
//! lifecycle is owned. This file is the "what gets sent / received"
//! contract.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One row in the agent input JSON's `connectors.enabled` array.
/// Mirrored on the agent side as `ConnectorSnapshotInput`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "camelCase")]
pub struct ConnectorTokenSnapshot {
    /// Connector id matching the agent-side `BUILTIN` factory map.
    pub id: String,
    /// Display email for the agent (used in error messages / logs).
    pub account_email: Option<String>,
    /// Scopes the user actually granted — agent uses this to decide
    /// which tools to register (skip tools whose `requires` aren't met).
    pub granted_scopes: Vec<String>,
    /// Fresh-at-spawn access_token. May expire during a long turn,
    /// in which case the agent emits a `host_request` (see below).
    pub access_token: String,
    /// RFC 3339 if known, otherwise None.
    pub expires_at: Option<String>,
}

/// Tagged-union of stdio host requests the agent can emit. Phase 1
/// only has one op; extending = adding a variant.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum HostRequest {
    RefreshConnectorToken {
        connector_id: String,
        request_id: String,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum HostResponse {
    RefreshConnectorToken {
        request_id: String,
        result: HostRefreshResult,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum HostRefreshResult {
    Ok {
        access_token: String,
        expires_at: Option<String>,
    },
    Err {
        message: String,
    },
}
