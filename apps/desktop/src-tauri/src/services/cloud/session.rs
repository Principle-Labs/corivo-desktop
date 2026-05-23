//! Cloud session capability.
//!
//! Public code only needs a narrow contract: fetch the per-user agent
//! gateway credentials, refresh them after an upstream auth failure, and
//! keep the managed model alias synced with the gateway. The concrete
//! Corivo token rotation implementation lives in the closed-source
//! private overlay.

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::Result;
use crate::services::model_catalog::{ModelCatalog, ModelDirectory};

#[derive(Debug, Clone)]
pub struct CloudAgentCreds {
    pub api_host: String,
    pub api_key: String,
    pub label: Option<String>,
    pub email: Option<String>,
}

#[async_trait]
pub trait CloudSessionService: Send + Sync {
    fn is_available(&self) -> bool {
        false
    }

    fn has_session(&self) -> Result<bool>;

    async fn fetch_agent_creds(&self) -> Result<CloudAgentCreds>;

    async fn refresh_agent_creds_now(&self) -> Result<()>;

    async fn refresh_model_directory(&self, catalog: Arc<ModelCatalog>) -> Result<ModelDirectory>;

    async fn set_active_model_alias(
        &self,
        catalog: Arc<ModelCatalog>,
        alias: String,
    ) -> Result<ModelDirectory>;

    async fn ensure_model_alias_synced(
        &self,
        catalog: Arc<ModelCatalog>,
        alias: String,
    ) -> Result<()>;
}
