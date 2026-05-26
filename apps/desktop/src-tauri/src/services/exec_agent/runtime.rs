use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::domain::config::{ApiShape, Config, ExecAgentAuthMode};
use crate::services::chatgpt_auth;
use crate::services::cloud::CloudSessionService;
use crate::services::config_service::ConfigService;
use crate::services::exec_agent::runner::CorivoAuth;
use crate::services::model_catalog::ModelCatalog;

const DEFAULT_COMPACTION_ANTHROPIC: &str = "claude-haiku-4-5";
const DEFAULT_COMPACTION_OPENAI: &str = "gpt-4o-mini";
/// Model id sent to `chatgpt.com/backend-api/codex/responses`. Both the
/// main turn and compaction use the same id — there is no separate
/// cheap-model path on that endpoint.
///
/// Empirical: trying `gpt-5-codex` (what the public Codex CLI ships
/// against the same endpoint) returns
/// "The 'gpt-5-codex' model is not supported when using Codex with a
/// ChatGPT account." — that variant is reserved for OpenAI's internal /
/// API-key flows. `gpt-5.4` is the id the ChatGPT-subscription Codex
/// path actually accepts.
const CHATGPT_MODEL_ID: &str = "gpt-5.4";
/// Refresh the ChatGPT access token if it expires within this window.
/// 5 minutes matches what Codex CLI uses internally.
const CHATGPT_REFRESH_LEEWAY: Duration = Duration::from_secs(5 * 60);

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
    /// Cheap-model partner the sidecar uses for history compaction.
    /// CorivoProxy + BYOK use per-shape defaults (haiku for anthropic,
    /// gpt-4o-mini for openai); Chatgpt mode reuses the main
    /// `gpt-5-codex` because chatgpt.com/backend-api/codex doesn't
    /// expose a cheaper sibling.
    pub compaction_model_id: String,
}

/// Resolve the auth bundle + model binding for a user-facing turn.
///
/// CorivoProxy mode reads the active alias from the model catalog and syncs
/// that alias into the gateway. BYOK mode keeps the thread's frozen model
/// binding, but still reads the configured key/base URL. Chatgpt mode
/// hardcodes `gpt-5-codex` + Responses API and transparently refreshes the
/// stored access_token via `config_service` when it's near expiry.
pub async fn resolve_user_turn_runtime(
    cfg: &Config,
    session: Option<Arc<dyn CloudSessionService>>,
    catalog: Option<Arc<ModelCatalog>>,
    config_service: Option<Arc<ConfigService>>,
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
            compaction_model_id: default_compaction_for_shape(thread_api_shape),
        }),
        ExecAgentAuthMode::Chatgpt => resolve_chatgpt_runtime(cfg, config_service).await,
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
    config_service: Option<Arc<ConfigService>>,
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
                compaction_model_id: default_compaction_for_shape(api_shape),
            })
        }
        ExecAgentAuthMode::Chatgpt => resolve_chatgpt_runtime(cfg, config_service).await,
    }
}

/// Per-shape compaction defaults (spec §7.6). Kept as a free function so
/// callers that don't go through `resolve_*_runtime` (none today, but the
/// surface is small enough that we leave it usable) can still pick the
/// right cheap partner.
pub fn default_compaction_for_shape(api_shape: ApiShape) -> String {
    match api_shape {
        ApiShape::Anthropic => DEFAULT_COMPACTION_ANTHROPIC.to_string(),
        ApiShape::Openai | ApiShape::OpenaiResponses => DEFAULT_COMPACTION_OPENAI.to_string(),
    }
}

/// Legacy entry point retained for callers that still want a
/// shape-only compaction pick. New code should read
/// `RuntimeBinding::compaction_model_id` instead — that's where the
/// Chatgpt branch overrides the shape default with the codex id.
#[deprecated(note = "read RuntimeBinding::compaction_model_id instead")]
pub fn resolve_compaction_partner(api_shape: ApiShape) -> String {
    default_compaction_for_shape(api_shape)
}

/// Chatgpt mode resolution. Reads `Config.exec_agent.chatgpt`,
/// transparently refreshes a near-expiry access_token, writes the
/// refreshed creds back to config, and returns a `RuntimeBinding`
/// pinned to `gpt-5-codex` on the Responses API shape.
///
/// Error cases:
///   * not signed in → "请先在设置中完成 ChatGPT 登录"
///   * refresh_token rejected → "ChatGPT 凭证已失效,请重新登录"
///   * config write failed → bubbles the IO error (rare; config.json
///     is the same file Corivo writes every settings flip to).
async fn resolve_chatgpt_runtime(
    cfg: &Config,
    config_service: Option<Arc<ConfigService>>,
) -> Result<RuntimeBinding, String> {
    let chatgpt = &cfg.exec_agent.chatgpt;
    if !chatgpt.is_signed_in() {
        return Err("ChatGPT 模式未登录 —— 请到设置页完成『使用 ChatGPT 登录』".to_string());
    }
    let account_id = chatgpt
        .account_id
        .clone()
        .ok_or_else(|| "ChatGPT 凭证缺少 account_id —— 请重新登录".to_string())?;

    let needs_refresh = chatgpt
        .expires_at
        .as_deref()
        .map(|s| is_near_expiry(s, CHATGPT_REFRESH_LEEWAY))
        .unwrap_or(false);

    let access_token = if needs_refresh {
        let refresh_token = chatgpt
            .refresh_token
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| {
                "ChatGPT 访问令牌过期但没有 refresh_token —— 请重新登录".to_string()
            })?;
        tracing::info!(
            target: "chatgpt_auth",
            expires_at = chatgpt.expires_at.as_deref().unwrap_or(""),
            "chatgpt_auth.token.refresh_attempt"
        );
        let session = chatgpt_auth::refresh_session(refresh_token).await.map_err(|e| {
            tracing::warn!(
                target: "chatgpt_auth",
                error = %e,
                "chatgpt_auth.token.refresh_failed"
            );
            format!("ChatGPT 凭证刷新失败,请重新登录: {e}")
        })?;
        let new_access = session.access_token.clone();
        if let Some(cfg_svc) = config_service.as_ref() {
            let mut next = cfg_svc.get();
            next.exec_agent.chatgpt = session.into_config();
            cfg_svc.update(next).map_err(|e| {
                format!("ChatGPT 凭证刷新成功但写入 config 失败: {e}")
            })?;
        } else {
            tracing::warn!(
                target: "chatgpt_auth",
                "chatgpt_auth.token.refresh_skipped_persist (no config_service)"
            );
        }
        new_access
    } else {
        chatgpt
            .access_token
            .clone()
            .ok_or_else(|| "ChatGPT 模式未登录".to_string())?
    };

    Ok(RuntimeBinding {
        auth: CorivoAuth::Chatgpt {
            access_token,
            account_id,
        },
        model_id: CHATGPT_MODEL_ID.to_string(),
        api_shape: ApiShape::OpenaiResponses,
        alias: None,
        cloud_session: None,
        compaction_model_id: CHATGPT_MODEL_ID.to_string(),
    })
}

fn is_near_expiry(expires_at_rfc3339: &str, leeway: Duration) -> bool {
    let Ok(parsed) = DateTime::parse_from_rfc3339(expires_at_rfc3339) else {
        // Unparseable timestamp — assume expired so the refresh path
        // either heals it or surfaces a fresh-login prompt.
        return true;
    };
    let when = parsed.with_timezone(&Utc);
    let now = Utc::now();
    let leeway_chrono = chrono::Duration::from_std(leeway).unwrap_or(chrono::Duration::zero());
    when <= now + leeway_chrono
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
    let compaction_model_id = default_compaction_for_shape(entry.client_protocol);
    Ok(RuntimeBinding {
        auth: CorivoAuth::CorivoProxy {
            gateway_url: creds.api_host,
            api_key: creds.api_key,
        },
        model_id: entry.upstream_model,
        api_shape: entry.client_protocol,
        alias: Some(entry.alias),
        cloud_session: Some(session),
        compaction_model_id,
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
        let binding = resolve_background_runtime(&byok_config(), None, None, None)
            .await
            .expect("BYOK background tasks should not require cloud credentials");

        assert_eq!(binding.model_id, "gpt-5.5");
        assert_eq!(binding.api_shape, ApiShape::OpenaiResponses);
        assert_eq!(binding.alias, None);
        assert!(binding.cloud_session.is_none());
        assert_eq!(binding.compaction_model_id, "gpt-4o-mini");
        match binding.auth {
            CorivoAuth::Byok { base_url, api_key } => {
                assert_eq!(base_url.as_deref(), Some("https://llm.example/v1"));
                assert_eq!(api_key, "sk-test");
            }
            CorivoAuth::CorivoProxy { .. } | CorivoAuth::Chatgpt { .. } => {
                panic!("expected BYOK auth")
            }
        }
    }

    #[tokio::test]
    async fn user_turn_runtime_preserves_byok_thread_binding() {
        let binding = resolve_user_turn_runtime(
            &byok_config(),
            None,
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
        assert_eq!(binding.compaction_model_id, "claude-haiku-4-5");
    }

    #[tokio::test]
    async fn chatgpt_mode_without_credentials_errors_with_login_prompt() {
        let mut cfg = Config::default();
        cfg.exec_agent.auth_mode = ExecAgentAuthMode::Chatgpt;
        let result =
            resolve_user_turn_runtime(&cfg, None, None, None, "x", ApiShape::OpenaiResponses)
                .await;
        match result {
            Err(msg) => assert!(msg.contains("ChatGPT"), "unexpected message: {msg}"),
            Ok(_) => panic!("Chatgpt mode without creds must surface a login-required error"),
        }
    }
}
