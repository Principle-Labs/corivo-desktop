//! JSON-RPC handler for the `save_note` native AgentTool
//! (memory-system-spec §3.4.1, §10.3).
//!
//! Wire shape (params):
//!   `{ content: string, scope?: "global"|"project"|"session",
//!      scope_ref?: string, source?: "user_explicit"|"agent_inferred",
//!      source_message_id?: string, confidence?: number,
//!      reason?: string }`
//!
//! Defaults:
//!   * `scope = "global"`
//!   * `source = "user_explicit"` — the prompt rule asks the agent to
//!     mark agent-inferred guesses explicitly via `source`. The handler
//!     respects whatever the agent sends; the safer default is treating
//!     omitted `source` as "user told me explicitly" because the prompt
//!     only triggers `save_note` after recognizing an explicit "记住X" /
//!     "以后都Y" pattern.
//!   * `source_thread_id` = the current turn's thread id (RpcDeps).

#[cfg(any(unix, windows))]
use serde_json::{json, Value};

#[cfg(any(unix, windows))]
use crate::db::repos::notes::{NewNote, NotesRepo};
#[cfg(any(unix, windows))]
use crate::domain::note::{NoteScope, NoteSourceType};
#[cfg(any(unix, windows))]
use crate::error::{CorivoError, Result};

// Wired into rpc_server::dispatch only, which is the per-turn RPC server
// for the corivo-agent sidecar. The server uses UDS on unix and named
// pipes on Windows; either way the handler is dead code on platforms
// that lack both (e.g. WASM CI).
#[cfg(any(unix, windows))]
pub async fn save_note_handler(
    params: Value,
    notes: Option<&dyn NotesRepo>,
    thread_id: &str,
) -> Result<Value> {
    let repo = notes.ok_or_else(|| {
        CorivoError::Internal("save_note: notes repo not initialized".to_string())
    })?;
    let content = params
        .get("content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CorivoError::Internal("save_note: missing 'content'".to_string()))?
        .to_string();
    let scope = params
        .get("scope")
        .and_then(Value::as_str)
        .map(|s| {
            NoteScope::parse(s)
                .ok_or_else(|| CorivoError::Internal(format!("save_note: bad scope '{s}'")))
        })
        .transpose()?
        .unwrap_or(NoteScope::Global);
    let scope_ref = params
        .get("scope_ref")
        .and_then(Value::as_str)
        .map(str::to_string);
    let source_type = params
        .get("source")
        .or_else(|| params.get("source_type"))
        .and_then(Value::as_str)
        .map(|s| {
            NoteSourceType::parse(s)
                .ok_or_else(|| CorivoError::Internal(format!("save_note: bad source '{s}'")))
        })
        .transpose()?
        .unwrap_or(NoteSourceType::UserExplicit);
    let source_message_id = params
        .get("source_message_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let confidence = params
        .get("confidence")
        .and_then(Value::as_f64)
        .map(|v| v.clamp(0.0, 1.0) as f32);

    let note = repo
        .create(NewNote {
            content,
            scope,
            scope_ref,
            source_type,
            source_message_id,
            source_thread_id: Some(thread_id.to_string()),
            confidence,
            status: None,
            expires_at: None,
        })
        .await?;

    Ok(json!({
        "id": note.id,
        "scope": note.scope.as_str(),
        "source_type": note.source_type.as_str(),
        "status": note.status.as_str(),
        "saved": true,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::test_in_memory_pool;
    use crate::db::repos::notes::SqliteNotesRepo;

    const SCHEMA_SQL: &str = include_str!("../../db/schema.sql");

    fn init_repo() -> SqliteNotesRepo {
        let pool = test_in_memory_pool().unwrap();
        pool.get().unwrap().execute_batch(SCHEMA_SQL).unwrap();
        SqliteNotesRepo::new(pool)
    }

    #[tokio::test]
    async fn user_explicit_default_writes_active_global_note() {
        let repo = init_repo();
        let result = save_note_handler(json!({ "content": "用中文回我" }), Some(&repo), "T1")
            .await
            .unwrap();
        assert_eq!(result["status"], "active");
        assert_eq!(result["scope"], "global");
        assert_eq!(result["source_type"], "user_explicit");
    }

    #[tokio::test]
    async fn agent_inferred_writes_suggested_note() {
        let repo = init_repo();
        let result = save_note_handler(
            json!({
                "content": "可能喜欢 jj",
                "source": "agent_inferred",
            }),
            Some(&repo),
            "T1",
        )
        .await
        .unwrap();
        assert_eq!(result["status"], "suggested");
        assert_eq!(result["source_type"], "agent_inferred");
    }

    #[tokio::test]
    async fn missing_content_errors() {
        let repo = init_repo();
        let err = save_note_handler(json!({}), Some(&repo), "T1").await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn session_scope_persists_thread_anchor() {
        let repo = init_repo();
        save_note_handler(
            json!({ "content": "本 thread 只讨论性能", "scope": "session" }),
            Some(&repo),
            "T1",
        )
        .await
        .unwrap();
        let rows = repo
            .list(crate::db::repos::notes::ListNotesOptions {
                scope: Some(NoteScope::Session),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source_thread_id.as_deref(), Some("T1"));
    }
}
