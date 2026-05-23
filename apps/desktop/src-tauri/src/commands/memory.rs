//! Tauri command surface for the memory system (memory-system-spec §3 / §10.5).
//!
//! Step 1a exposes the four lifecycle ops over `notes` plus a lookup
//! by id. The list command takes the same filter shape as the repo so
//! the Settings UI can drive scope / status filters directly without a
//! second IPC schema.

use std::sync::Arc;

use tauri::State;

use crate::commands::config::AppState;
use crate::db::repos::notes::{ListNotesOptions, NewNote, UpdateNote};
use crate::domain::chat::{ChatThread, SystemTaskKind};
use crate::domain::note::{Note, NoteScope, NoteSourceType, NoteStatus};
use crate::services::background_agent_task::ScheduleEnqueue;
use crate::services::persona::{renderer as persona_renderer, PersonaDistillTask};
use crate::services::session_learner::SessionLearnerTask;
use tauri::{AppHandle, Manager};

fn require_repo(
    state: &State<'_, AppState>,
) -> Result<std::sync::Arc<dyn crate::db::repos::notes::NotesRepo>, String> {
    state
        .notes_repo
        .as_ref()
        .cloned()
        .ok_or_else(|| "notes repo not initialized".to_string())
}

#[tauri::command]
pub async fn list_notes(
    state: State<'_, AppState>,
    scope: Option<NoteScope>,
    scope_ref: Option<String>,
    status: Option<NoteStatus>,
    source_type: Option<NoteSourceType>,
    limit: Option<u32>,
) -> Result<Vec<Note>, String> {
    require_repo(&state)?
        .list(ListNotesOptions {
            scope,
            scope_ref,
            status,
            source_type,
            limit,
        })
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_note(state: State<'_, AppState>, id: String) -> Result<Option<Note>, String> {
    require_repo(&state)?
        .by_id(&id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_note(
    state: State<'_, AppState>,
    content: String,
    scope: Option<NoteScope>,
    scope_ref: Option<String>,
) -> Result<Note, String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err("note content cannot be empty".to_string());
    }
    require_repo(&state)?
        .create(NewNote {
            content: trimmed.to_string(),
            scope: scope.unwrap_or(NoteScope::Global),
            scope_ref,
            // The Settings UI is always the user speaking — never
            // surface an "agent inferred" path from this command.
            source_type: NoteSourceType::UserExplicit,
            source_message_id: None,
            source_thread_id: None,
            confidence: None,
            status: None,
            expires_at: None,
        })
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_note(
    state: State<'_, AppState>,
    id: String,
    content: Option<String>,
    status: Option<NoteStatus>,
    confidence: Option<f32>,
) -> Result<Note, String> {
    require_repo(&state)?
        .update(
            &id,
            UpdateNote {
                content,
                status,
                confidence,
                superseded_by: None,
            },
        )
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_note(state: State<'_, AppState>, id: String) -> Result<(), String> {
    require_repo(&state)?
        .delete(&id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_system_threads(
    state: State<'_, AppState>,
    task: Option<SystemTaskKind>,
    limit: Option<u32>,
) -> Result<Vec<ChatThread>, String> {
    let threads = state
        .chat_threads
        .as_ref()
        .ok_or_else(|| "chat threads repo not initialized".to_string())?
        .clone();
    threads
        .list_system(task, limit.unwrap_or(50))
        .await
        .map_err(|e| e.to_string())
}

/// Reveal `<app_data_dir>/logs/background-tasks/` in the user's file
/// manager (Finder on macOS) so they can read the per-run log files
/// directly. Creates the directory first if it doesn't exist yet (the
/// background tasks may not have written anything since boot).
#[tauri::command]
pub async fn open_background_task_logs_dir(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir unavailable: {e}"))?
        .join("logs")
        .join("background-tasks");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return Err(format!("mkdir failed: {e}"));
    }
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| format!("open failed: {e}"))?;
    Ok(())
}

#[tauri::command]
pub async fn get_auto_persona(app: AppHandle) -> Result<String, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir unavailable: {e}"))?;
    Ok(persona_renderer::read(&dir).unwrap_or_default())
}

#[tauri::command]
pub async fn regenerate_auto_persona(state: State<'_, AppState>) -> Result<(), String> {
    let scheduler = state
        .bg_scheduler
        .as_ref()
        .ok_or_else(|| "background scheduler not initialized".to_string())?
        .clone();
    scheduler.enqueue(Arc::new(PersonaDistillTask::new(30)));
    Ok(())
}

/// Manually enqueue a session learner run against `thread_id` —
/// mostly a debug entry point so you don't have to wait 25 minutes
/// for the idle hook to fire when verifying the pipeline.
#[tauri::command]
pub async fn rerun_session_learner(
    state: State<'_, AppState>,
    thread_id: String,
) -> Result<(), String> {
    let scheduler = state
        .bg_scheduler
        .as_ref()
        .ok_or_else(|| "background scheduler not initialized".to_string())?
        .clone();
    scheduler.enqueue(Arc::new(SessionLearnerTask::new(thread_id, None)));
    Ok(())
}
