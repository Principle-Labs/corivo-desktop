//! IPC surface for the `/workflows` page (v1430).
//!
//! Read paths (`workflows_list` / `workflows_list_runs` / `workflows_get_run`)
//! are pure queries. Write paths (`workflows_save` / `workflows_delete` /
//! `workflows_set_enabled`) touch both the filesystem (WORKFLOW.md under
//! `$APPDATA/corivo/workflows/<slug>/`) and the SQLite tables; the
//! order is deliberate — file first, then row — so a partial failure
//! leaves a readable file behind for inspection rather than a phantom
//! schedule pointing at nothing.
//!
//! `workflows_preview_trigger` is a pure function exposed so the
//! create/edit drawer can show "下次将在 …" without round-tripping
//! through a save.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tauri::State;
use ts_rs::TS;

use crate::commands::config::AppState;
use crate::db::time::now_utc;
use crate::domain::ipc_error::TauriError;
use crate::domain::workflow::{
    Trigger, WorkflowDefinition, WorkflowNotifyPolicy, WorkflowRun, WorkflowSchedule,
    WorkflowScheduleSource,
};
use crate::services::scheduled_workflows::UpsertSchedule;

/// What the `/workflows` list page renders. Pairs each on-disk
/// definition with its (optional) schedule row. Workflows that exist
/// on disk but have never been scheduled appear with `schedule: null`.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
pub struct WorkflowView {
    pub definition: WorkflowDefinition,
    pub schedule: Option<WorkflowSchedule>,
}

/// Create-or-update payload. The UI passes the same shape for both —
/// slug uniquely identifies the workflow (immutable post-creation).
#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
pub struct WorkflowSaveSpec {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub tool_whitelist: Vec<String>,
    #[ts(type = "number")]
    pub max_turns: u32,
    pub system_prompt: String,
    pub trigger: Trigger,
    pub enabled: bool,
    /// v1432 — notification policy chosen in the drawer. Defaults to
    /// `Always` when the field is omitted by an older client.
    #[serde(default)]
    pub notify_policy: WorkflowNotifyPolicy,
}

/// Pure validator output used by the trigger picker. `next_run_at`
/// is `None` either because the trigger is malformed (then `error`
/// is set) or because the trigger has no future firings (e.g. a
/// `Once` whose instant already passed).
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
pub struct TriggerPreview {
    pub valid: bool,
    pub next_run_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Read paths
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn workflows_list(
    state: State<'_, AppState>,
) -> Result<Vec<WorkflowView>, TauriError> {
    let store = workflow_store(&state)?;
    let definitions = store.scan_definitions();
    let schedules = store.list_schedules().await.map_err(internal)?;
    let mut out = Vec::with_capacity(definitions.len());
    for def in definitions {
        let schedule = schedules.iter().find(|s| s.slug == def.slug).cloned();
        out.push(WorkflowView {
            definition: def,
            schedule,
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn workflows_list_runs(
    state: State<'_, AppState>,
    slug: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<WorkflowRun>, TauriError> {
    let store = workflow_store(&state)?;
    store
        .list_runs(slug, limit.unwrap_or(50))
        .await
        .map_err(internal)
}

#[tauri::command]
pub async fn workflows_get_run(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<WorkflowRun>, TauriError> {
    let store = workflow_store(&state)?;
    store.get_run(id).await.map_err(internal)
}

#[tauri::command]
pub async fn workflows_preview_trigger(
    trigger: Trigger,
) -> Result<TriggerPreview, TauriError> {
    match trigger.validate() {
        Err(error) => Ok(TriggerPreview {
            valid: false,
            next_run_at: None,
            error: Some(error),
        }),
        Ok(()) => Ok(TriggerPreview {
            valid: true,
            next_run_at: trigger.next_after(now_utc()),
            error: None,
        }),
    }
}

// ---------------------------------------------------------------------------
// Write paths
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn workflows_save(
    state: State<'_, AppState>,
    spec: WorkflowSaveSpec,
) -> Result<WorkflowView, TauriError> {
    validate_slug(&spec.slug).map_err(invalid)?;
    if spec.name.trim().is_empty() {
        return Err(invalid("workflow 名称不能为空"));
    }
    if spec.system_prompt.trim().is_empty() {
        return Err(invalid("system prompt 不能为空"));
    }
    if spec.max_turns == 0 {
        return Err(invalid("max_turns 必须 ≥ 1"));
    }
    spec.trigger.validate().map_err(invalid)?;

    let store = workflow_store(&state)?;
    let definition = WorkflowDefinition {
        slug: spec.slug.clone(),
        name: spec.name.trim().to_string(),
        description: spec.description.and_then(|s| {
            let trimmed = s.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        }),
        tool_whitelist: spec
            .tool_whitelist
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        max_turns: spec.max_turns,
        system_prompt: spec.system_prompt,
        notify_policy: spec.notify_policy,
    };

    store.write_definition(&definition).map_err(internal)?;
    let schedule = store
        .upsert_schedule(UpsertSchedule {
            slug: spec.slug.clone(),
            trigger: spec.trigger,
            enabled: spec.enabled,
            source: WorkflowScheduleSource::User,
            created_by_thread_id: None,
            notify_policy: spec.notify_policy,
        })
        .await
        .map_err(internal)?;
    Ok(WorkflowView {
        definition,
        schedule: Some(schedule),
    })
}

#[tauri::command]
pub async fn workflows_delete(
    state: State<'_, AppState>,
    slug: String,
) -> Result<(), TauriError> {
    let store = workflow_store(&state)?;
    // Schedule row first — workflow_runs cascades, the file purge
    // can't fail in a way that leaves a dangling schedule.
    store.delete_schedule(slug.clone()).await.map_err(internal)?;
    store.delete_definition(&slug).map_err(internal)?;
    Ok(())
}

#[tauri::command]
pub async fn workflows_set_enabled(
    state: State<'_, AppState>,
    slug: String,
    enabled: bool,
) -> Result<WorkflowView, TauriError> {
    let store = workflow_store(&state)?;
    store
        .set_enabled(slug.clone(), enabled)
        .await
        .map_err(internal)?;
    let definition = store
        .load_definition(&slug)
        .ok_or_else(|| invalid(format!("workflow `{slug}` 缺少 WORKFLOW.md")))?;
    let schedule = store.get_schedule(&slug).await.map_err(internal)?;
    Ok(WorkflowView {
        definition,
        schedule,
    })
}

#[tauri::command]
pub async fn workflows_run_now(
    state: State<'_, AppState>,
    slug: String,
) -> Result<String, TauriError> {
    let ticker = state
        .workflow_ticker
        .as_ref()
        .ok_or_else(|| not_ready("workflow_ticker"))?
        .clone();
    ticker.run_now(slug).await.map_err(internal)
}

/// Cancel a running OR queued workflow by slug. Returns `true` if
/// anything was actually stopped (running future aborted, or queued
/// task pulled from the queue). Cleanup (release dedup claim, write
/// the failure run row, fire `workflow:completed`) happens via the
/// task's `on_dispatch_aborted` hook — the IPC just signals the
/// scheduler and returns.
#[tauri::command]
pub async fn workflows_cancel_run(
    state: State<'_, AppState>,
    slug: String,
) -> Result<bool, TauriError> {
    let scheduler = state
        .bg_scheduler
        .as_ref()
        .ok_or_else(|| not_ready("bg_scheduler"))?
        .clone();
    Ok(scheduler.cancel(&slug).await)
}

#[tauri::command]
pub async fn workflows_acknowledge_run(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<(), TauriError> {
    let store = workflow_store(&state)?;
    store.acknowledge_run(run_id).await.map_err(internal)
}

#[tauri::command]
pub async fn workflows_unread_count(
    state: State<'_, AppState>,
) -> Result<u32, TauriError> {
    let store = workflow_store(&state)?;
    store.unread_count().await.map_err(internal)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn workflow_store(
    state: &State<'_, AppState>,
) -> Result<std::sync::Arc<crate::services::scheduled_workflows::WorkflowStore>, TauriError> {
    state
        .workflow_store
        .as_ref()
        .cloned()
        .ok_or_else(|| not_ready("workflow_store"))
}

fn internal<E: std::fmt::Display>(error: E) -> TauriError {
    TauriError::Unknown {
        message: error.to_string(),
    }
}

fn invalid(message: impl Into<String>) -> TauriError {
    TauriError::Unknown {
        message: message.into(),
    }
}

fn not_ready(component: &str) -> TauriError {
    TauriError::Unknown {
        message: format!("{component} not initialized"),
    }
}

/// Slug rule: lowercase ASCII letters, digits, hyphens; non-empty;
/// no leading/trailing hyphen; max 64 chars. Strict on purpose —
/// the slug doubles as a filesystem directory name and a JSON object
/// key, so anything Unicode-y would land us in normalization hell.
fn validate_slug(slug: &str) -> Result<(), String> {
    if slug.is_empty() {
        return Err("slug 不能为空".into());
    }
    if slug.len() > 64 {
        return Err("slug 长度不能超过 64".into());
    }
    if slug.starts_with('-') || slug.ends_with('-') {
        return Err("slug 不能以连字符开头或结尾".into());
    }
    if !slug
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err("slug 只能包含小写字母、数字、连字符".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_accepts_kebab() {
        assert!(validate_slug("daily-review").is_ok());
        assert!(validate_slug("a").is_ok());
        assert!(validate_slug("workflow-1").is_ok());
    }

    #[test]
    fn slug_rejects_bad_input() {
        assert!(validate_slug("").is_err());
        assert!(validate_slug("Daily-Review").is_err());
        assert!(validate_slug("-leading").is_err());
        assert!(validate_slug("trailing-").is_err());
        assert!(validate_slug("under_score").is_err());
        assert!(validate_slug("空格 不行").is_err());
    }
}
