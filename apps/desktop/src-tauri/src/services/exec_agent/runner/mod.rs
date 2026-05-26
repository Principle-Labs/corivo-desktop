//! Single entry point that spawns the `corivo-agent` sidecar for one
//! user turn (spec §5).

pub mod corivo;

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::Notify;

use crate::domain::config::{ApiShape, ThinkingLevel};
use crate::error::{CorivoError, Result};
use crate::services::cloud::CloudSessionService;
use crate::services::connector::ConnectorTokenSnapshot;
use crate::services::exec_agent::rpc_server::RpcDeps;
use crate::services::recall::stream::StreamEmitter;

pub use corivo::FocusContextInput;

/// Inputs the corivo runner needs in addition to the universal
/// `(thread_id, user_content, emitter)` triple. Bundled here so
/// `commands::exec_agent` can keep its dispatch site small.
///
/// Spec §8.2: the thread's permanent model binding is plumbed through
/// here so the runner can build §5.1 SidecarInput without re-reading
/// the Settings cache (which may be stale).
pub struct CorivoRunInput {
    pub auth: CorivoAuth,
    pub thread_model_id: String,
    pub thread_api_shape: ApiShape,
    /// Reasoning budget the sidecar forwards on the main turn (spec
    /// §5.1 `model.thinking_level`). Snapshotted from
    /// `Config.exec_agent.thinking_level` at the start of the turn so
    /// a Settings flip mid-turn can't split one response across two
    /// budgets.
    pub thinking_level: ThinkingLevel,
    /// Compaction model id paired with the main model — looked up
    /// from the §7.5 catalog (CorivoProxy mode) or defaulted per
    /// shape (BYOK mode, spec §7.6).
    pub compaction_model_id: String,
    pub focus_context: Option<FocusContextInput>,
    /// Spec §8.1: jsonl session store root for pi-coding-agent's
    /// `SessionManager`. Resolved by the command layer from
    /// `app.path().app_data_dir()` — typically
    /// `$APPDATA/corivo-agent-sessions/`. The runner ensures the
    /// directory exists before spawning the sidecar.
    pub sessions_dir: PathBuf,
    /// App-private bundled skills root. In packaged builds this is
    /// `<resource_dir>/bundled-skills/` (shipped via Tauri
    /// `bundle.resources`); in `app:dev` it falls back to the repo
    /// source `apps/desktop/bundled-skills/`. `None` means the
    /// resource isn't on disk — the sidecar then falls back to
    /// user-only skill sources.
    pub bundled_skills_dir: Option<PathBuf>,
    /// Snapshot of enabled + connected third-party connectors. Each
    /// entry carries a fresh `access_token` (just refreshed by
    /// `ConnectorRegistry::enabled_snapshot` if it was close to
    /// expiry). Token TTL is typically 1h — long enough for almost
    /// every turn. Mid-turn refresh is Phase 2 work (UDS bridge);
    /// today, if a turn outlasts the TTL the connector tool just
    /// gets 401 back and surfaces the error to the model.
    pub connectors_snapshot: Vec<ConnectorTokenSnapshot>,
    /// Per-server JSON entries for `mcpServer`-shape connectors. Empty
    /// when no MCP-backed connector is enabled+connected. Each entry
    /// matches `packages/agent/src/types.ts::McpServerSpec` and gets
    /// dropped verbatim into `input.tools.mcp_servers`. Tokens live in
    /// each entry's `token_cache_dir`, not inline — the sidecar's
    /// mcporter reconnects against the cache directory directly.
    pub mcp_server_specs: Vec<Value>,
    pub deps: RpcDeps,
}

/// Three authentication paths the corivo-agent sidecar supports
/// (spec §7.3 / §7.4):
///
/// * `CorivoProxy` — the user logged into Corivo; we forward the
///   gateway base URL + bearer token. The backend routes to the real
///   provider behind the scenes.
/// * `Byok` — the user supplied their own provider key. `base_url`
///   is `Some` when overriding the SDK default (e.g. for local
///   Ollama).
/// * `Chatgpt` — "Sign in with ChatGPT" subscription auth (mirrors
///   Codex CLI). The sidecar hits `chatgpt.com/backend-api/codex/responses`
///   with a Bearer access_token plus the `ChatGPT-Account-Id` header.
#[derive(Clone)]
pub enum CorivoAuth {
    /// Closed-beta path. The cloud auth flow returns a per-user
    /// `(apiHost, apiKey)` pair in addition to the local session
    /// token. The desktop talks
    /// **directly** to `apiHost` using `apiKey` — the corivo backend is
    /// NOT a forward proxy on the chat hot path; it just brokers
    /// upstream provider credentials per user. Both fields come from
    /// `CloudSessionService::fetch_agent_creds().await`.
    CorivoProxy {
        /// Anthropic-compatible upstream URL (e.g. an account-pool
        /// endpoint hosted on the user's behalf). Goes into the §5.1
        /// `auth.base_url` field; pi-ai uses it as `Model.baseUrl`.
        gateway_url: String,
        /// Upstream provider key issued by the corivo backend for this
        /// user — NOT the corivo session token. Goes into `auth.token`,
        /// which pi-ai forwards as the `x-api-key` header (Anthropic
        /// shape) or `Authorization: Bearer ...` (OpenAI shape).
        api_key: String,
    },
    Byok {
        base_url: Option<String>,
        api_key: String,
    },
    /// ChatGPT subscription path. The sidecar uses `access_token` as a
    /// Bearer credential and `account_id` as the `ChatGPT-Account-Id`
    /// header on every Responses API call. Refreshed by
    /// `services::chatgpt_auth` ~5 min before expiry; runtime
    /// resolution short-circuits with `not_signed_in` if either field
    /// is empty.
    Chatgpt {
        access_token: String,
        account_id: String,
    },
}

impl CorivoAuth {
    /// Cheap pre-flight: do we hold any credential at all? Used to
    /// short-circuit a runner call with a friendly error before we
    /// spawn the sidecar.
    pub fn has_credentials(&self) -> bool {
        match self {
            Self::CorivoProxy { api_key, .. } => !api_key.trim().is_empty(),
            Self::Byok { api_key, .. } => !api_key.trim().is_empty(),
            Self::Chatgpt {
                access_token,
                account_id,
            } => !access_token.trim().is_empty() && !account_id.trim().is_empty(),
        }
    }

    /// Short label suitable for `tracing` fields; never includes
    /// secret material.
    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::CorivoProxy { .. } => "corivo_proxy",
            Self::Byok { .. } => "byok",
            Self::Chatgpt { .. } => "chatgpt",
        }
    }

    /// Serialize into the §5.1 `auth` JSON object the sidecar parses.
    pub fn to_json(&self) -> Value {
        match self {
            Self::CorivoProxy {
                gateway_url,
                api_key,
            } => json!({
                "mode": "corivo_proxy",
                "base_url": gateway_url,
                "token": api_key,
            }),
            Self::Byok { base_url, api_key } => json!({
                "mode": "byok",
                "base_url": base_url.clone().map(Value::String).unwrap_or(Value::Null),
                "token": api_key,
            }),
            Self::Chatgpt {
                access_token,
                account_id,
            } => json!({
                "mode": "chatgpt",
                "base_url": "https://chatgpt.com/backend-api/codex",
                "token": access_token,
                "account_id": account_id,
            }),
        }
    }
}

/// Single dispatch entry the Tauri command layer calls. Routes one
/// turn through the corivo-agent sidecar.
///
/// `cancel_signal` is registered in `AppState::pending_turns` keyed by
/// `thread_id`; firing `notify_one()` on it preempts the runner and
/// finishes with [`crate::services::recall::stream::FinishReason::Cancelled`].
pub async fn run_turn(
    thread_id: &str,
    user_content: String,
    input: CorivoRunInput,
    extra_system_prompt: Option<String>,
    emitter: &StreamEmitter,
    cancel_signal: Arc<Notify>,
    // `Some` in CorivoProxy mode — lets the runner react to a sidecar
    // `auth_failed` event by forcing a cloud-session refresh so the
    // next turn picks up fresh api_key/apiHost. `None` in BYOK mode (no
    // corivo session to refresh; sidecar 401 surfaces to the user as
    // a generic upstream error).
    cloud_session: Option<Arc<dyn CloudSessionService>>,
) -> Result<()> {
    corivo::run_turn(
        thread_id,
        user_content,
        input.auth,
        input.thread_model_id,
        input.thread_api_shape,
        input.thinking_level,
        input.compaction_model_id,
        extra_system_prompt,
        input.focus_context,
        input.sessions_dir,
        input.bundled_skills_dir,
        input.connectors_snapshot,
        input.mcp_server_specs,
        input.deps,
        emitter,
        cancel_signal,
        cloud_session,
    )
    .await
    .map_err(|e: CorivoError| e)
}
