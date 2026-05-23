//! JSON-RPC handlers for read-only memory-system tools surfaced to the
//! sidecar (memory-system-spec §10.3 Step 1b / Step 3).
//!
//! Day-1 implementations:
//!
//! * `chat_thread_get` — return the full message log for a thread
//!   (filtered to `kind='user'` so background tasks can't pull other
//!   tasks' threads).
//! * `note_list` — list notes with simple filters (scope, status).
//! * `thread_search` — FTS5 hits on `chat_threads.summary` +
//!   `summary_topics`, scored with a 60-day recency half-life.
//!
//! All three are read-only. The `save_note` writer lives in
//! `save_note_handler.rs` and is gated to user-facing turns.

use serde_json::{json, Value};

use std::path::Path;
use std::sync::Arc;

use crate::db::repos::chat::{ChatMessageRepo, ChatThreadRepo};
use crate::db::repos::frames::FrameRepo;
use crate::db::repos::notes::{ListNotesOptions, NotesRepo};
use crate::domain::chat::ChatRole;
use crate::domain::note::{NoteScope, NoteSourceType, NoteStatus};
use crate::error::{CorivoError, Result};
use crate::services::memory::{MemoryItem, MemoryService, RecallRequest};

pub async fn chat_thread_get_handler(
    params: Value,
    threads: Option<&dyn ChatThreadRepo>,
    messages: Option<&dyn ChatMessageRepo>,
) -> Result<Value> {
    let threads = threads
        .ok_or_else(|| CorivoError::Internal("chat_threads repo not initialized".to_string()))?;
    let messages = messages
        .ok_or_else(|| CorivoError::Internal("chat_messages repo not initialized".to_string()))?;
    let thread_id = params
        .get("thread_id")
        .and_then(Value::as_str)
        .ok_or_else(|| CorivoError::Internal("chat_thread_get: missing 'thread_id'".to_string()))?
        .to_string();
    let after_id = params
        .get("after_message_id")
        .and_then(Value::as_str)
        .map(str::to_string);

    let thread = threads
        .by_id(&thread_id)
        .await?
        .ok_or_else(|| CorivoError::Internal(format!("thread {thread_id} not found")))?;
    if !matches!(thread.kind, crate::domain::chat::ChatThreadKind::User) {
        return Err(CorivoError::Internal(format!(
            "thread {thread_id} is not a user thread (kind={})",
            thread.kind.as_str()
        )));
    }
    let msgs = messages.by_thread(&thread_id).await?;

    // Take everything strictly after `after_message_id` (exclusive) when
    // provided. Used by the session learner to read only the unlearned
    // window without redoing previously-summarized turns.
    let mut started = after_id.is_none();
    let rendered: Vec<Value> = msgs
        .into_iter()
        .filter(|m| {
            if started {
                true
            } else {
                if Some(&m.id) == after_id.as_ref() {
                    started = true;
                }
                false
            }
        })
        .map(|m| {
            json!({
                "id": m.id,
                "role": match m.role {
                    ChatRole::User => "user",
                    ChatRole::Assistant => "assistant",
                    ChatRole::System => "system",
                },
                "content_text": m.content_text,
                "created_at": m.created_at,
            })
        })
        .collect();

    Ok(json!({
        "thread_id": thread.id,
        "title": thread.title,
        "messages": rendered,
        "summary": thread.summary,
        "summary_updated_at": thread.summary_updated_at,
    }))
}

pub async fn note_list_handler(params: Value, notes: Option<&dyn NotesRepo>) -> Result<Value> {
    let repo =
        notes.ok_or_else(|| CorivoError::Internal("notes repo not initialized".to_string()))?;
    let scope = params
        .get("scope")
        .and_then(Value::as_str)
        .map(|s| {
            NoteScope::parse(s)
                .ok_or_else(|| CorivoError::Internal(format!("note_list: bad scope '{s}'")))
        })
        .transpose()?;
    let status = params
        .get("status")
        .and_then(Value::as_str)
        .map(|s| {
            NoteStatus::parse(s)
                .ok_or_else(|| CorivoError::Internal(format!("note_list: bad status '{s}'")))
        })
        .transpose()?;
    let source_type = params
        .get("source")
        .or_else(|| params.get("source_type"))
        .and_then(Value::as_str)
        .map(|s| {
            NoteSourceType::parse(s)
                .ok_or_else(|| CorivoError::Internal(format!("note_list: bad source '{s}'")))
        })
        .transpose()?;
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .map(|v| v as u32);

    let rows = repo
        .list(ListNotesOptions {
            scope,
            scope_ref: None,
            status,
            source_type,
            limit,
        })
        .await?;
    let rendered: Vec<Value> = rows
        .into_iter()
        .map(|n| {
            json!({
                "id": n.id,
                "content": n.content,
                "scope": n.scope.as_str(),
                "source_type": n.source_type.as_str(),
                "status": n.status.as_str(),
                "confidence": n.confidence,
                "created_at": n.created_at,
            })
        })
        .collect();
    Ok(json!({ "notes": rendered }))
}

/// `memory_search` native tool (memory-system-spec §7.7 tool-level).
///
/// Re-runs the same `MemoryService::recall` pipeline the system-level
/// injection uses, but driven by the agent's mid-turn query instead
/// of the user's original message. Returns flat hits across all three
/// layers so the model can decide which one to follow up on.
pub async fn memory_search_handler(
    params: serde_json::Value,
    frames: Option<Arc<dyn FrameRepo>>,
    notes: Option<Arc<dyn NotesRepo>>,
    messages: Option<Arc<dyn ChatMessageRepo>>,
    current_thread_id: &str,
) -> Result<serde_json::Value> {
    let (frames, notes, messages) = match (frames, notes, messages) {
        (Some(f), Some(n), Some(m)) => (f, n, m),
        _ => {
            return Err(CorivoError::Internal(
                "memory_search: backing repos not initialized".to_string(),
            ))
        }
    };
    let query = params
        .get("query")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CorivoError::Internal("memory_search: missing 'query'".to_string()))?
        .to_string();
    let layers: Option<Vec<String>> = params.get("layers").and_then(|v| v.as_array()).map(|arr| {
        arr.iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect()
    });

    let service = MemoryService::new(frames, notes, messages);
    let mut request = RecallRequest::new(query);
    request.thread_id = Some(current_thread_id.to_string());
    let result = service.recall(&request).await?;

    let layer_filter = |item: &MemoryItem| -> bool {
        let Some(layers) = &layers else { return true };
        let kind = match item {
            MemoryItem::Frame { .. } => "frame",
            MemoryItem::Note { .. } => "note",
            MemoryItem::Message { .. } => "message",
        };
        layers.iter().any(|l| l == kind)
    };
    let hits: Vec<serde_json::Value> = result
        .items
        .into_iter()
        .filter(layer_filter)
        .map(|item| match item {
            MemoryItem::Frame {
                id,
                excerpt,
                score,
                ts,
                app_name,
                window_title,
            } => {
                json!({
                    "kind": "frame",
                    "id": id,
                    "excerpt": excerpt,
                    "score": score,
                    "ts": ts,
                    "app_name": app_name,
                    "window_title": window_title,
                })
            }
            MemoryItem::Note {
                id,
                content,
                score,
                scope,
                source_type,
                status,
            } => json!({
                "kind": "note",
                "id": id,
                "content": content,
                "score": score,
                "scope": scope.as_str(),
                "source_type": source_type.as_str(),
                "status": status.as_str(),
            }),
            MemoryItem::Message {
                id,
                thread_id,
                content,
                score,
                ts,
            } => json!({
                "kind": "message",
                "id": id,
                "thread_id": thread_id,
                "content": content,
                "score": score,
                "ts": ts,
            }),
        })
        .collect();
    Ok(json!({
        "hits": hits,
        "diagnostics": {
            "frame_hits": result.diagnostics.frame_hits,
            "note_hits": result.diagnostics.note_hits,
            "message_hits": result.diagnostics.message_hits,
            "elapsed_ms": result.diagnostics.elapsed_ms,
        }
    }))
}

pub async fn auto_persona_get_previous_handler(
    _params: serde_json::Value,
    app_data_dir: &Path,
) -> Result<serde_json::Value> {
    let path = app_data_dir.join("auto-persona.md");
    let exists = path.exists();
    let content = if exists {
        std::fs::read_to_string(&path)
            .map_err(|e| CorivoError::Internal(format!("read auto-persona.md 失败: {e}")))?
    } else {
        String::new()
    };
    Ok(json!({
        "exists": exists,
        "path": path.display().to_string(),
        "content": content,
    }))
}

pub async fn thread_search_handler(
    params: Value,
    threads: Option<&dyn ChatThreadRepo>,
) -> Result<Value> {
    let threads = threads
        .ok_or_else(|| CorivoError::Internal("chat_threads repo not initialized".to_string()))?;
    let query = params
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CorivoError::Internal("thread_search: missing 'query'".to_string()))?;
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(5)
        .min(20) as u32;

    // Tokenize the query for FTS5 (CJK-friendly via jieba). Empty token
    // list short-circuits to no hits.
    let tokens = crate::services::tokenize::tokenize_for_query(query);
    let Some(fts_query) = tokens else {
        return Ok(json!({ "hits": [] }));
    };

    let hits = threads.search_summaries(&fts_query, limit).await?;
    let rendered: Vec<Value> = hits
        .into_iter()
        .map(|h| {
            json!({
                "thread_id": h.thread_id,
                "title": h.title,
                "summary": h.summary,
                "summary_updated_at": h.summary_updated_at,
                "created_at": h.created_at,
                "score": h.score,
            })
        })
        .collect();
    Ok(json!({ "hits": rendered }))
}
