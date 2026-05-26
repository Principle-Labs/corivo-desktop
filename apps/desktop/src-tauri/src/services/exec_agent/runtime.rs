use std::sync::Arc;

use crate::domain::config::{ApiShape, Config, ExecAgentAuthMode};
use crate::services::cloud::CloudSessionService;
use crate::services::exec_agent::runner::CorivoAuth;
use crate::services::model_catalog::ModelCatalog;

const DEFAULT_COMPACTION_ANTHROPIC: &str = "claude-haiku-4-5";
const DEFAULT_COMPACTION_OPENAI: &str = "gpt-4o-mini";

#[derive(Clone)]
pub struct RuntimeBinding {
    pub auth: CorivoAuth,
    /// What gets sent to the upstream adapter as the model field.
    pub model_id: String,
    /// Drives the sidecar's adapter pick + the compaction partner fallback.
    pub api_shape: ApiShape,
    /// Directory alias that drove this turn. `None` in BYOK mode.
    pub alias: Option<String>,
    /// `Some` only in CorivoProxy mode, where auth failures should refresh
    /// Corivo-managed credentials. BYOK failures belong to the user's key.
    pub cloud_session: Option<Arc<dyn CloudSessionService>>,
}

/// Resolve the auth bundle + model binding for a user-facing turn.
///
/// CorivoProxy mode reads the active alias from the model catalog and syncs
/// that alias into the gateway. BYOK mode keeps the thread's frozen model
/// binding, but still reads the configured key/base URL.
pub async fn resolve_user_turn_runtime(
    cfg: &Config,
    session: Option<Arc<dyn CloudSessionService>>,
    catalog: Option<Arc<ModelCatalog>>,
    thread_upstream_model: &str,
    thread_api_shape: ApiShape,
) -> Result<RuntimeBinding, String> {
    match cfg.exec_agent.auth_mode {
        ExecAgentAuthMode::CorivoProxy => resolve_corivo_proxy_runtime(session, catalog).await,
        ExecAgentAuthMode::Byok => Ok(RuntimeBinding {
            auth: resolve_byok_auth(cfg)?,
            model_id: thread_upstream_model.to_string(),
            api_shape: thread_api_shape,
            alias: None,
            cloud_session: None,
        }),
    }
}

/// Resolve the auth bundle + model binding for a background agent task.
///
/// Background tasks create a fresh hidden system thread at run time, so BYOK
/// mode uses the current Settings model instead of a pre-existing thread row.
pub async fn resolve_background_runtime(
    cfg: &Config,
    session: Option<Arc<dyn CloudSessionService>>,
    catalog: Option<Arc<ModelCatalog>>,
) -> Result<RuntimeBinding, String> {
    match cfg.exec_agent.auth_mode {
        ExecAgentAuthMode::CorivoProxy => resolve_corivo_proxy_runtime(session, catalog).await,
        ExecAgentAuthMode::Byok => {
            let model_id = trimmed_required(
                cfg.exec_agent.byok_model.as_deref(),
                "BYOK 模式但未配置 model id —— 请到设置页填写",
            )?;
            let api_shape = cfg
                .exec_agent
                .byok_api_shape
                .ok_or_else(|| "BYOK 模式但未选择 api shape —— 请到设置页填写".to_string())?;
            Ok(RuntimeBinding {
                auth: resolve_byok_auth(cfg)?,
                model_id,
                api_shape,
                alias: None,
                cloud_session: None,
            })
        }
    }
}

/// Resolve the compaction model id for a turn. The current model directory
/// shape does not surface a per-alias compaction partner, so a Corivo-wide
/// default per protocol is good enough while the catalog is small.
pub fn resolve_compaction_partner(api_shape: ApiShape) -> String {
    match api_shape {
        ApiShape::Anthropic => DEFAULT_COMPACTION_ANTHROPIC.to_string(),
        ApiShape::Openai | ApiShape::OpenaiResponses => DEFAULT_COMPACTION_OPENAI.to_string(),
    }
}

async fn resolve_corivo_proxy_runtime(
    session: Option<Arc<dyn CloudSessionService>>,
    catalog: Option<Arc<ModelCatalog>>,
) -> Result<RuntimeBinding, String> {
    let session = session.ok_or_else(|| "cloud session not initialized".to_string())?;
    let catalog = catalog
        .ok_or_else(|| "model directory not initialized — restart after login".to_string())?;
    let entry = catalog.active_entry().ok_or_else(|| {
        "model directory is empty — refresh after the admin enables at least one model".to_string()
    })?;
    let creds = session
        .fetch_agent_creds()
        .await
        .map_err(|e| e.to_string())?;
    session
        .ensure_model_alias_synced(catalog.clone(), entry.alias.clone())
        .await
        .map_err(|e| format!("failed to align gateway with alias {}: {}", entry.alias, e))?;
    tracing::info!(
        target: "exec_agent",
        api_host = %creds.api_host,
        alias = %entry.alias,
        upstream_model = %entry.upstream_model,
        api_shape = ?entry.client_protocol,
        api_key_len = creds.api_key.len(),
        label = ?creds.label,
        email = ?creds.email,
        "exec_agent.corivo_proxy.creds_resolved"
    );
    Ok(RuntimeBinding {
        auth: CorivoAuth::CorivoProxy {
            gateway_url: creds.api_host,
            api_key: creds.api_key,
        },
        model_id: entry.upstream_model,
        api_shape: entry.client_protocol,
        alias: Some(entry.alias),
        cloud_session: Some(session),
    })
}

fn resolve_byok_auth(cfg: &Config) -> Result<CorivoAuth, String> {
    let api_key = trimmed_required(
        cfg.exec_agent.byok_key.as_deref(),
        "BYOK mode is selected but no API key is configured",
    )?;
    Ok(CorivoAuth::Byok {
        base_url: cfg
            .exec_agent
            .byok_base_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        api_key,
    })
}

fn trimmed_required(value: Option<&str>, message: &str) -> Result<String, String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::config::{ApiShape, Config, ExecAgentAuthMode};
    use crate::services::exec_agent::CorivoAuth;

    fn byok_config() -> Config {
        let mut cfg = Config::default();
        cfg.exec_agent.auth_mode = ExecAgentAuthMode::Byok;
        cfg.exec_agent.byok_key = Some(" sk-test ".to_string());
        cfg.exec_agent.byok_base_url = Some(" https://llm.example/v1 ".to_string());
        cfg.exec_agent.byok_model = Some(" gpt-5.5 ".to_string());
        cfg.exec_agent.byok_api_shape = Some(ApiShape::OpenaiResponses);
        cfg
    }

    #[tokio::test]
    async fn background_runtime_uses_byok_config_without_catalog_or_cloud_session() {
        let binding = resolve_background_runtime(&byok_config(), None, None)
            .await
            .expect("BYOK background tasks should not require cloud credentials");

        assert_eq!(binding.model_id, "gpt-5.5");
        assert_eq!(binding.api_shape, ApiShape::OpenaiResponses);
        assert_eq!(binding.alias, None);
        assert!(binding.cloud_session.is_none());
        match binding.auth {
            CorivoAuth::Byok { base_url, api_key } => {
                assert_eq!(base_url.as_deref(), Some("https://llm.example/v1"));
                assert_eq!(api_key, "sk-test");
            }
            CorivoAuth::CorivoProxy { .. } => panic!("expected BYOK auth"),
        }
    }

    #[tokio::test]
    async fn user_turn_runtime_preserves_byok_thread_binding() {
        let binding = resolve_user_turn_runtime(
            &byok_config(),
            None,
            None,
            "thread-frozen-model",
            ApiShape::Anthropic,
        )
        .await
        .expect("BYOK user turns should use thread binding plus configured key");

        assert_eq!(binding.model_id, "thread-frozen-model");
        assert_eq!(binding.api_shape, ApiShape::Anthropic);
        assert!(binding.cloud_session.is_none());
    }
}
