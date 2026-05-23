//! Tauri command surface for the connector framework.
//!
//! Generic by design — every command takes a `connector_id: String`
//! parameter so adding Notion / Slack / Calendar later doesn't add new
//! commands, just new manifest.json files (and new factories on the
//! agent side).
//!
//! Frontend calls:
//!   - `connectors_list` on Settings → Integrations mount
//!   - `connector_enable(id)` when the user toggles a card on
//!   - `connector_disable(id)` when they toggle it off (keeps tokens)
//!   - `connector_connect(id)` when they click "Connect <Provider>"
//!   - `connector_disconnect(id)` when they click "Disconnect"
//!
//! `connect` is long-running (opens the system browser, waits up to
//! 5 min for the user to complete consent). The frontend should show a
//! spinner and keep the button disabled until the promise resolves.
//!
//! All commands delegate to `state.cloud.connectors.*`. In the
//! open-source build the trait points at `NoopConnectorsService`: list
//! returns an empty Vec (so the Integrations tab is a clean empty
//! state) and every mutation returns `FeatureUnavailable`. The tab
//! itself is hidden via the frontend capability probe.

use tauri::State;

use crate::commands::config::AppState;
use crate::domain::ipc_error::TauriError;
use crate::services::connector::ConnectorSummary;

#[tauri::command]
pub async fn connectors_list(
    state: State<'_, AppState>,
) -> Result<Vec<ConnectorSummary>, TauriError> {
    Ok(state.cloud.connectors.list().await?)
}

#[tauri::command]
pub async fn connector_enable(
    state: State<'_, AppState>,
    id: String,
) -> Result<ConnectorSummary, TauriError> {
    Ok(state.cloud.connectors.enable(id).await?)
}

#[tauri::command]
pub async fn connector_disable(state: State<'_, AppState>, id: String) -> Result<(), TauriError> {
    state.cloud.connectors.disable(id).await?;
    Ok(())
}

#[tauri::command]
pub async fn connector_connect(
    state: State<'_, AppState>,
    id: String,
) -> Result<ConnectorSummary, TauriError> {
    Ok(state.cloud.connectors.connect(id).await?)
}

/// Bulk connect: open ONE OAuth flow whose scope is the union of every
/// listed connector's manifest scopes, then bind + enable all of them
/// on the resulting account. Used by the Settings → Integrations
/// `ProviderCard` so checking three Google products and clicking
/// connect opens the browser exactly once. All `connector_ids` must
/// share `manifest.auth.provider == provider`; mismatch is rejected
/// upstream.
#[tauri::command]
pub async fn connector_provider_connect(
    state: State<'_, AppState>,
    provider: String,
    connector_ids: Vec<String>,
) -> Result<Vec<ConnectorSummary>, TauriError> {
    Ok(state
        .cloud
        .connectors
        .provider_connect(provider, connector_ids)
        .await?)
}

/// Install + authorize an `mcpServer`-shape connector (Linear, future
/// Notion/GitHub MCP, …). Same UX commitment as `connector_connect`:
/// long-running (opens the system browser, waits up to 5 min for
/// consent). The frontend ConnectorCard dispatches to this command
/// instead of `connector_connect` when the manifest's `auth.type` is
/// `"mcpServer"`.
#[tauri::command]
pub async fn connector_install_mcp(
    state: State<'_, AppState>,
    id: String,
) -> Result<ConnectorSummary, TauriError> {
    Ok(state.cloud.connectors.install_mcp(id).await?)
}

#[tauri::command]
pub async fn connector_disconnect(
    state: State<'_, AppState>,
    id: String,
) -> Result<ConnectorSummary, TauriError> {
    Ok(state.cloud.connectors.disconnect(id).await?)
}
