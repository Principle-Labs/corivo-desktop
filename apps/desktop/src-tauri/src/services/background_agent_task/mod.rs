//! Background agent task framework (memory-system-spec §11).
//!
//! Where it sits in the system:
//!
//! ```text
//! scheduler.rs               runner.rs                       sidecar
//! ─────────────              ──────────                      ───────
//! daily tick / idle hook     ┌────────────────────────┐     spawn corivo-agent
//!     │                      │ BackgroundAgentTaskRunner│ →   (system_prompt_extra
//!     ▼                      │                        │      = task.system_prompt())
//! FIFO queue + semaphore  →  │ • insert kind='system' │     (initial user message
//! (single in-flight task)    │   chat_thread          │      = task.initial_user_message())
//!                            │ • build SidecarInput   │     (tools.native filtered
//!                            │   with whitelisted     │       to task.tool_whitelist())
//!                            │   tools, discard       │
//!                            │   StreamEmitter        │
//!                            │ • run via shared       │
//!                            │   exec_agent runner    │
//!                            │ • accumulate text →    │
//!                            │   consume_output()     │
//!                            └────────────────────────┘
//! ```
//!
//! The task itself is just a trait the spec describes (§11.4): kind,
//! system_prompt, initial_user_message, tool_whitelist, max_turns,
//! consume_output. Each task type lives in its own module
//! (e.g. `services::session_learner::task`) and implements the trait.
//!
//! Two important invariants vs. user-facing turns:
//!
//! * **Visibility**: `kind='system'` threads are filtered out of every
//!   user-facing list query. The sidecar gets the thread_id as
//!   `session_id` like any turn, so the jsonl session store still
//!   accumulates a real trace under
//!   `${sessions_dir}/<thread_id>.jsonl`.
//! * **Tools**: the task supplies a whitelist of native tool names; the
//!   runner intersects it with the full registry. Writes (`save_note`)
//!   are not in any task's whitelist by design — all side effects flow
//!   through `consume_output`, where the Rust side validates the
//!   agent's reply before mutating any store.

pub mod log;
pub mod runner;
pub mod scheduler;

use async_trait::async_trait;
use std::sync::Arc;

use crate::commands::config::AppState;
use crate::db::repos::chat::{ChatMessageRepo, ChatThreadRepo};
use crate::db::repos::notes::NotesRepo;
use crate::domain::chat::SystemTaskKind;
use crate::domain::config::{ApiShape, ThinkingLevel};
use crate::error::Result;
use crate::services::cloud::CloudSessionService;
use crate::services::exec_agent::runtime::{
    resolve_background_runtime, resolve_compaction_partner,
};
use crate::services::exec_agent::CorivoAuth;
use std::path::PathBuf;

pub use runner::TaskOutcome;
pub use scheduler::{BackgroundAgentScheduler, ScheduleEnqueue};

/// Inputs every task needs to actually run a sidecar turn. Filled in
/// by `BackgroundAgentScheduler` from `AppState` so individual tasks
/// don't have to plumb State around.
///
/// `Clone` is required so the scheduler can `tauri::async_runtime::spawn`
/// the runner with an owned copy (the spawned future needs `'static`).
/// Every field is either `Copy`, `Arc`-backed, or a cheap `Clone`
/// (`PathBuf` / `String`), so cloning is effectively pointer copies.
#[derive(Clone)]
pub struct TaskDeps {
    pub db_pool: crate::db::pool::DbPool,
    pub chat_threads: Arc<dyn ChatThreadRepo>,
    pub chat_messages: Arc<dyn ChatMessageRepo>,
    pub notes_repo: Arc<dyn NotesRepo>,
    pub frames_repo: Arc<dyn crate::db::repos::frames::FrameRepo>,
    pub auth: CorivoAuth,
    pub model_id: String,
    pub api_shape: ApiShape,
    pub thinking_level: ThinkingLevel,
    pub compaction_model_id: String,
    pub sessions_dir: PathBuf,
    pub app_data_dir: PathBuf,
    pub bundled_skills_dir: Option<PathBuf>,
    pub bridge_pending: crate::services::exec_agent::mcp_bridge::PendingMap,
    pub app: tauri::AppHandle<tauri::Wry>,
    /// Live cloud session handle. `Some` only in CorivoProxy mode so an
    /// `auth_failed` from the sidecar can refresh Corivo-managed
    /// credentials. BYOK background tasks pass `None` because the user's
    /// upstream key must fail directly.
    pub cloud_session: Option<Arc<dyn CloudSessionService>>,
    /// v1511 — `WorkflowStore` handle exposed so background tasks that
    /// invoke `schedule_task` (e.g. session learner proposing a
    /// recurring review) can reach the same write path the user-facing
    /// agent uses. `None` only when AppState was wired without the
    /// workflows service.
    pub workflow_store: Option<Arc<crate::services::scheduled_workflows::WorkflowStore>>,
}

impl TaskDeps {
    /// Build TaskDeps from AppState. Returns `None` when the app is
    /// still mid-boot (a repo / session / model catalog isn't ready);
    /// the scheduler treats that as "skip this tick, try next interval".
    pub async fn from_state(state: &AppState, app: tauri::AppHandle<tauri::Wry>) -> Option<Self> {
        Self::try_from_state(state, app).await.ok()
    }

    pub async fn try_from_state(
        state: &AppState,
        app: tauri::AppHandle<tauri::Wry>,
    ) -> std::result::Result<Self, String> {
        let chat_threads = state
            .chat_threads
            .as_ref()
            .cloned()
            .ok_or_else(|| "chat_threads_not_initialized".to_string())?;
        let chat_messages = state
            .chat_messages
            .as_ref()
            .cloned()
            .ok_or_else(|| "chat_messages_not_initialized".to_string())?;
        let notes_repo = state
            .notes_repo
            .as_ref()
            .cloned()
            .ok_or_else(|| "notes_repo_not_initialized".to_string())?;
        let frames_repo = state
            .frames_repo
            .as_ref()
            .cloned()
            .ok_or_else(|| "frames_repo_not_initialized".to_string())?;
        // Pending map lives on AppState directly now; the UDS bridge is
        // optional and only relevant for external corivo-mcp clients.
        let bridge_pending = state.permission_pending.clone();
        let cfg = state.config_service.get();
        let db_pool = state.db.pool();
        let app_data_dir = tauri::Manager::path(&app)
            .app_data_dir()
            .map_err(|e| format!("app_data_dir_failed: {e}"))?;
        let sessions_dir = app_data_dir.join("corivo-agent-sessions");
        let bundled_skills_dir = tauri::Manager::path(&app)
            .resource_dir()
            .ok()
            .map(|d| d.join("bundled-skills"))
            .filter(|p| p.is_dir());
        let runtime = resolve_background_runtime(
            &cfg,
            Some(state.cloud.session.clone()),
            state.model_catalog.as_ref().cloned(),
        )
        .await
        .map_err(|e| format!("runtime_not_ready: {e}"))?;
        let compaction_model_id = resolve_compaction_partner(runtime.api_shape);
        let workflow_store = state.workflow_store.as_ref().cloned();
        Ok(TaskDeps {
            db_pool,
            chat_threads,
            chat_messages,
            notes_repo,
            frames_repo,
            auth: runtime.auth,
            model_id: runtime.model_id,
            api_shape: runtime.api_shape,
            thinking_level: cfg.exec_agent.thinking_level,
            compaction_model_id,
            sessions_dir,
            app_data_dir,
            bundled_skills_dir,
            bridge_pending,
            app,
            cloud_session: runtime.cloud_session,
            workflow_store,
        })
    }
}

/// memory-system-spec §11.4 contract.
///
/// Implementors live in `services::session_learner` etc. The trait is
/// async only on `consume_output` because that's the only step that
/// hits the database; the others are pure config lookups.
#[async_trait]
pub trait BackgroundAgentTask: Send + Sync {
    fn kind(&self) -> SystemTaskKind;
    fn system_prompt(&self) -> String;
    fn initial_user_message(&self) -> String;
    /// Native tool names the task is allowed to call. Runtime-loaded
    /// tasks (e.g. user-authored scheduled workflows) need an owned
    /// `Vec<String>`; the runner intersects with the registry so
    /// extras are silently dropped.
    fn tool_whitelist(&self) -> Vec<String>;
    fn max_turns(&self) -> u32 {
        20
    }
    /// Called once at the start of `runner::run`, after the system
    /// `chat_threads` row is created but before the sidecar spawns.
    /// Default no-op. Scheduled-workflow tasks override to emit a
    /// `workflow:started` Tauri event so the UI can show a spinner
    /// during the (often slow) sidecar turn.
    fn before_run(&self, _deps: &TaskDeps, _thread_id: &str) {}
    /// Called when the scheduler abandons the task before `runner::run`
    /// could complete — e.g. `deps_not_ready` retries exhausted, the
    /// runner returned `Err` before `consume_output` could clean up,
    /// or the user clicked Cancel and the spawned future was aborted.
    /// Default no-op. Scheduled-workflow tasks override to release
    /// the dedup slug + emit a `workflow:completed` failure event so
    /// the user actually sees that the run died, instead of staring
    /// at an indefinite spinner.
    fn on_dispatch_aborted(&self, _reason: &str) {}
    /// Identifier the scheduler's `cancel(key)` method matches against
    /// to interrupt a running or queued task. Default `None` =
    /// non-cancellable (session_learner / persona_distill etc.: the
    /// user has no UI surface to cancel them anyway). Scheduled-
    /// workflow tasks override to return their slug so the
    /// `workflows_cancel_run` IPC can find them.
    fn cancellation_key(&self) -> Option<String> {
        None
    }
    /// Process the sidecar's final assistant message. The runner
    /// already stripped streaming framing; you get the plain text.
    async fn consume_output(
        &self,
        output: String,
        outcome: &TaskOutcome,
        deps: &TaskDeps,
    ) -> Result<()>;
}
