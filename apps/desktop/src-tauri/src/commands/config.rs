use std::collections::HashMap;
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Runtime, State};
use tokio::sync::{Mutex, Notify};

use crate::{
    db::{
        repos::{
            chat::{ChatMessageRepo, ChatThreadRepo},
            frames::FrameRepo,
            notes::NotesRepo,
        },
        Database,
    },
    domain::config::{Config, ConfigChanged},
    services::{
        background_agent_task::BackgroundAgentScheduler,
        capture_client::CaptureClient,
        capture_pipeline::CapturePipeline,
        capture_store::CaptureStore,
        cloud::CloudServices,
        config_service::ConfigService,
        connector::ConnectorRegistry,
        exec_agent::{mcp_bridge::PendingMap, McpBridge},
        hotkey::HotkeyService,
        model_catalog::ModelCatalog,
        privacy_filter::PrivacyFilter,
        scheduled_workflows::{ScheduledWorkflowTicker, WorkflowStore},
        skill_share::SkillShareService,
    },
};

/// Central state plumbing (v300, frames-as-truth).
///
/// Step 5 deliberately leaves `capture_pipeline` and `frames_repo`
/// `Option`s — Step 6 wires them in once the schema flip to v300 lands.
/// Until then commands that need them surface a "not yet ready" error.
pub struct AppState {
    pub db: Arc<Database>,
    pub capture_store: Arc<CaptureStore>,
    pub config_service: Arc<ConfigService>,

    // v3 wiring — populated in Step 6.
    pub capture_pipeline: Option<Arc<CapturePipeline>>,
    pub frames_repo: Option<Arc<dyn FrameRepo>>,

    /// Phase 6: cross-platform capture helper sidecar. `None` when the
    /// helper failed to spawn at boot (degraded mode); commands that
    /// depend on it should bail with a clear error rather than crashing.
    /// Most call sites read it via [`crate::services::capture_client::global::get`]
    /// instead of going through AppState — this field exists so the
    /// shutdown path can drop the client cleanly.
    pub capture_client: Option<Arc<CaptureClient>>,

    // Chat thread / message persistence (used by chat + Quick Ask + the
    // corivo-agent-backed chat turn).
    pub chat_threads: Option<Arc<dyn ChatThreadRepo>>,
    pub chat_messages: Option<Arc<dyn ChatMessageRepo>>,

    /// Promoted declarative memory store (memory-system-spec §3). `None`
    /// only on boot races; commands that need it bail with a clear error.
    pub notes_repo: Option<Arc<dyn NotesRepo>>,

    // Phase 5 Quick Ask wiring.
    pub hotkey_service: Option<Arc<HotkeyService>>,

    // Long-lived MCP bridge (UDS server) — answers requests from
    // external `corivo-mcp` clients. None when the bind fails at boot
    // OR on platforms without AF_UNIX (Windows). The corivo-agent path
    // no longer needs this; `permission_pending` below carries the
    // permission-resolution channel independently of the bridge.
    pub exec_agent_bridge: Option<Arc<McpBridge>>,

    /// Permission-resolution channel shared by the corivo-agent per-turn
    /// rpc_server, the (optional) external-client MCP bridge, and the
    /// `exec_agent_permission_reply` Tauri command. Lives on AppState
    /// (not on McpBridge) so the agent flow keeps working on platforms
    /// where the UDS bridge isn't running.
    pub permission_pending: PendingMap,

    // Bridges host-installed skills (~/.agents/skills/, ~/.claude/skills/)
    // into the per-app skills dir. Driven by
    // `ExecAgentConfig.skill_share.enabled`; reconciled at boot and on
    // every `set_config` write.
    pub skill_share: Option<Arc<SkillShareService>>,

    // Phase C §7.5: cached `/v1/models` list backing the Settings model
    // picker. Populated at boot from `$APPDATA/.models-cache.json` and
    // refreshed after login + when the picker mounts.
    pub model_catalog: Option<Arc<ModelCatalog>>,

    /// Third-party service connectors (Gmail, future Notion/Slack/...).
    /// Owns the manifest catalog, the in-memory access_token cache, and
    /// the per-connector refresh single-flight locks. Lazy-spawned at
    /// boot in `lib.rs::run` once `config_service` is ready.
    pub connector_registry: Option<Arc<ConnectorRegistry>>,

    /// FIFO scheduler for background agent tasks (memory-system-spec §11).
    /// `None` only when AppState wasn't fully wired (test fixtures /
    /// degraded boot). Commands queueing tasks bail with a clear error
    /// in that case.
    pub bg_scheduler: Option<Arc<BackgroundAgentScheduler>>,

    /// Scheduled-workflows store (v1430). Owns the filesystem dir under
    /// `$APPDATA/corivo/workflows/` and the `workflow_schedules` /
    /// `workflow_runs` repo. `commands::workflows::*` reads/writes
    /// through this handle; the [`ScheduledWorkflowTicker`] runs a
    /// 60 s loop against it.
    pub workflow_store: Option<Arc<WorkflowStore>>,
    /// Workflow ticker handle — exposed on AppState so the "立即运行"
    /// IPC reuses the same enqueue path as the cron fire.
    pub workflow_ticker: Option<ScheduledWorkflowTicker>,

    /// In-flight `exec_agent_send` turns keyed by `thread_id`. The
    /// command layer inserts an `Arc<Notify>` here on entry and removes
    /// it on exit; `exec_agent_cancel` fires `notify_one()` on the
    /// matching entry to abort the sidecar mid-stream. The runner's
    /// stdout pump races against this signal in a `tokio::select!` so
    /// cancellation pre-empts in-flight HTTP / tool calls instead of
    /// waiting for the next event boundary.
    pub pending_turns: Arc<Mutex<HashMap<String, Arc<Notify>>>>,

    /// Cloud capability bundle (auth / billing / models directory /
    /// connectors / telemetry / updater policy / session creds). Always
    /// non-null; in the open-source build every field is a noop that
    /// returns `CorivoError::FeatureUnavailable`. In the closed Corivo
    /// build (cargo feature `corivo-cloud`) each field gets a real
    /// implementation wrapping the corresponding private service.
    ///
    /// Command handlers call `state.cloud.<trait>.<method>()` directly.
    pub cloud: Arc<CloudServices>,

    /// Privacy filter (docs/privacy-filter-spec.md). 在 `lib.rs::run`
    /// 时从 `Config.privacy_filter` bootstrap;`secret` 类目永远 on,
    /// 总开关默认 off,等用户在 Settings 同意下载模型后才转 on。
    /// Always-Some 因为它没有 IPC/DB 依赖,纯内存对象 —— 测试 fixture
    /// 也能调 `PrivacyFilter::new_disabled()` 兜底。
    pub privacy_filter: Arc<PrivacyFilter>,
}

#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> Result<Config, String> {
    Ok(state.config_service.get())
}

#[tauri::command]
pub async fn set_config(
    app: AppHandle,
    state: State<'_, AppState>,
    config: Config,
) -> Result<(), String> {
    state.config_service.update(config).map_err(String::from)?;
    // Reconcile host-skill symlinks immediately so a check/uncheck in
    // the UI is visible to the next sidecar spawn without a restart.
    // Idempotent + best-effort: a sync failure shouldn't block the
    // user's config write (worst case the next boot re-reconciles).
    if let Some(skill_share) = state.skill_share.as_ref() {
        let cfg = state.config_service.get();
        if let Err(error) = skill_share.sync(&cfg.exec_agent.skill_share.enabled) {
            tracing::warn!(?error, "skill_share.sync_after_set_config_failed");
        }
    }
    // Broadcast so every webview (main + Quick Ask overlay) invalidates
    // its `["config"]` cache. The Quick Ask `I18nProvider` reads its
    // language from this cache, so without the broadcast a UI-language
    // flip in the main window wouldn't take effect in Quick Ask until
    // the overlay was reloaded.
    emit_config_changed(&app);
    // capture_pipeline config sync lands in Step 6 (when the pipeline
    // is actually mounted on AppState).
    Ok(())
}

fn emit_config_changed<R: Runtime>(app: &AppHandle<R>) {
    if let Err(error) = app.emit(ConfigChanged::EVENT, &()) {
        tracing::warn!(?error, "config.emit_changed_failed");
    }
}
