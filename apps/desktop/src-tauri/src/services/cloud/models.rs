//! Managed model directory.
//!
//! Closed impl: sub2api gateway behind `${gateway}/v1/me/models`,
//! returns the per-user alias list the operator (or "models_set_active"
//! call) has granted. Cached at `$APPDATA/.models-cache.json`.
//!
//! Open-source impl: there is no managed directory. The OSS Settings
//! picker reads `Config.exec_agent.anthropic_api_key` and shows a single
//! "self-hosted" pseudo-alias if the user has pasted a key. Switching
//! aliases is a no-op (only one is possible). `is_available()` returns
//! `false` so the picker UI can switch to a simpler shape.

use async_trait::async_trait;

use crate::error::Result;

pub use crate::services::model_catalog::ModelDirectory;

#[async_trait]
pub trait ModelsService: Send + Sync {
    /// True when the build has a real managed catalog (closed Corivo
    /// build). Open-source = false; the Settings picker should fall
    /// back to "用户自带 key" mode.
    fn is_available(&self) -> bool {
        false
    }

    /// Sync read of the cached directory. Always succeeds (returns an
    /// empty directory if the cache hasn't been populated yet) so the
    /// Settings UI never blocks on a first-mount network call.
    async fn get_available(&self) -> Result<ModelDirectory>;

    /// Force a fetch from the cloud + write-through to the on-disk
    /// cache. Errors propagate so the Settings UI can show a "刷新失败"
    /// toast.
    async fn refresh(&self) -> Result<ModelDirectory>;

    /// Switch the user's active alias. Blocks until the backend
    /// confirms the gateway-side group flip; returns the refreshed
    /// directory with `active_alias` updated.
    async fn set_active_alias(&self, alias: String) -> Result<ModelDirectory>;
}
