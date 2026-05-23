//! Cross-table memory recall (memory-system-spec §7).
//!
//! Two consumer surfaces:
//!
//! * `local_context.rs` calls [`MemoryService::recall`] every user turn
//!   and stitches the result into the prompt as `<relevant_memory>`.
//! * The `memory_search` native tool calls the same service when the
//!   agent decides it needs a more precise lookup mid-turn.
//!
//! Both surfaces query the same backend so a relevance tweak in the
//! scorer benefits everyone.

mod render;
mod scorer;

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::db::repos::chat::{ChatMessageRepo, MessageHit};
use crate::db::repos::frames::{FrameRepo, ListFramesOptions};
use crate::db::repos::notes::{NoteHit, NotesRepo};
use crate::domain::frame::Frame;
use crate::domain::note::{Note, NoteScope, NoteSourceType, NoteStatus};
use crate::error::Result;
use crate::services::tokenize;

pub use render::render_relevant_block;

/// Per-layer token caps (memory-system-spec §7.6). Rough approximation:
/// 1 token ≈ 3 chars (CJK-friendly).
#[derive(Debug, Clone, Copy)]
pub struct RecallBudget {
    pub notes_chars: usize,
    pub frames_chars: usize,
    pub messages_chars: usize,
}

impl Default for RecallBudget {
    fn default() -> Self {
        // notes 600t × 3 ≈ 1800 chars
        // frames 1000t × 3 ≈ 3000 chars
        // messages 500t × 3 ≈ 1500 chars
        Self {
            notes_chars: 1800,
            frames_chars: 3000,
            messages_chars: 1500,
        }
    }
}

/// One memory hit, unified across layers. Surfaces enough metadata for
/// the prompt renderer + the explainable UI (which layer this came from).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MemoryItem {
    Frame {
        id: String,
        excerpt: String,
        score: f32,
        ts: DateTime<Utc>,
        app_name: Option<String>,
        window_title: Option<String>,
    },
    Note {
        id: String,
        content: String,
        score: f32,
        scope: NoteScope,
        source_type: NoteSourceType,
        status: NoteStatus,
    },
    Message {
        id: String,
        thread_id: String,
        content: String,
        score: f32,
        ts: DateTime<Utc>,
    },
}

#[derive(Debug, Clone)]
pub struct RecallRequest {
    pub user_message: String,
    pub thread_id: Option<String>,
    pub project_id: Option<String>,
    pub budget: RecallBudget,
    /// Cap per layer BEFORE budget trimming. Defaults to 8.
    pub per_layer_top_n: usize,
}

impl RecallRequest {
    pub fn new(user_message: String) -> Self {
        Self {
            user_message,
            thread_id: None,
            project_id: None,
            budget: RecallBudget::default(),
            per_layer_top_n: 8,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct RecallDiagnostics {
    pub frame_hits: usize,
    pub note_hits: usize,
    pub message_hits: usize,
    pub elapsed_ms: u128,
}

#[derive(Debug, Default, Clone)]
pub struct RecallResult {
    pub items: Vec<MemoryItem>,
    pub diagnostics: RecallDiagnostics,
}

pub struct MemoryService {
    frames: Arc<dyn FrameRepo>,
    notes: Arc<dyn NotesRepo>,
    messages: Arc<dyn ChatMessageRepo>,
}

impl MemoryService {
    pub fn new(
        frames: Arc<dyn FrameRepo>,
        notes: Arc<dyn NotesRepo>,
        messages: Arc<dyn ChatMessageRepo>,
    ) -> Self {
        Self {
            frames,
            notes,
            messages,
        }
    }

    /// Returns the persistent-block notes (scope=global + active +
    /// user_explicit) — the memory layer doesn't render them but
    /// `local_context.rs` already calls into notes for this; exposed
    /// here so callers can use a single service handle.
    pub async fn persistent_global_notes(&self, limit: u32) -> Result<Vec<Note>> {
        use crate::db::repos::notes::ListNotesOptions;
        self.notes
            .list(ListNotesOptions {
                scope: Some(NoteScope::Global),
                status: Some(NoteStatus::Active),
                source_type: Some(NoteSourceType::UserExplicit),
                limit: Some(limit),
                ..Default::default()
            })
            .await
    }

    /// Single tool-level + system-level recall entry point. Empty /
    /// punctuation-only queries short-circuit to an empty result so
    /// the prompt doesn't get a wasted block.
    pub async fn recall(&self, req: &RecallRequest) -> Result<RecallResult> {
        let start = std::time::Instant::now();
        let Some(fts_query) = tokenize::tokenize_for_query(&req.user_message) else {
            return Ok(RecallResult::default());
        };

        let per_layer = req.per_layer_top_n.max(1) as u32;

        // Parallelize the three FTS lookups — they're all blocking on
        // the SQLite pool but disjoint queries.
        let frames_task = self.frames.fts_search(
            &req.user_message,
            ListFramesOptions {
                limit: per_layer,
                ..Default::default()
            },
        );
        let notes_task = self.notes.fts_search(
            &fts_query,
            // Skip suggested + archived rows — they're noise in recall.
            Some(NoteStatus::Active),
            None,
            per_layer,
        );
        let messages_task = self.messages.fts_search(&fts_query, per_layer);

        let (frames_res, notes_res, messages_res) =
            tokio::join!(frames_task, notes_task, messages_task);

        let frames_hits = frames_res.unwrap_or_default();
        let notes_hits = notes_res.unwrap_or_default();
        let messages_hits = messages_res.unwrap_or_default();

        let diagnostics = RecallDiagnostics {
            frame_hits: frames_hits.len(),
            note_hits: notes_hits.len(),
            message_hits: messages_hits.len(),
            elapsed_ms: start.elapsed().as_millis(),
        };

        // Re-score per layer with the spec §7.4 weights.
        let now = crate::db::time::now_utc();
        let mut items: Vec<MemoryItem> = Vec::new();

        let frame_items = score_and_pack_frames(frames_hits, now, req.budget.frames_chars);
        items.extend(frame_items);

        let note_items = score_and_pack_notes(
            notes_hits,
            now,
            req.budget.notes_chars,
            req.project_id.as_deref(),
        );
        items.extend(note_items);

        let message_items = score_and_pack_messages(
            messages_hits,
            now,
            req.budget.messages_chars,
            req.thread_id.as_deref(),
        );
        items.extend(message_items);

        Ok(RecallResult { items, diagnostics })
    }
}

fn score_and_pack_frames(
    hits: Vec<Frame>,
    now: DateTime<Utc>,
    budget_chars: usize,
) -> Vec<MemoryItem> {
    let half_life_days = 7.0_f32;
    let layer_weight = 1.0_f32;
    let mut scored: Vec<(f32, MemoryItem)> = hits
        .into_iter()
        .map(|f| {
            // FrameRepo::fts_search doesn't return raw BM25 today —
            // approximate score with recency only (frames already
            // ordered by score in the repo, so we keep that order).
            let ts = f.captured_at;
            let recency = scorer::recency_decay(now, ts, half_life_days);
            let score = recency * layer_weight;
            let primary = f
                .ax_text
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .or(f.ocr_text.as_deref())
                .unwrap_or("");
            let excerpt: String = primary.chars().take(200).collect();
            let item = MemoryItem::Frame {
                id: f.id,
                excerpt,
                score,
                ts,
                app_name: f.app_name,
                window_title: f.window_title,
            };
            (score, item)
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    pack_within_budget(scored, budget_chars, |item| match item {
        MemoryItem::Frame { excerpt, .. } => excerpt.chars().count() + 80,
        _ => 0,
    })
}

fn score_and_pack_notes(
    hits: Vec<NoteHit>,
    now: DateTime<Utc>,
    budget_chars: usize,
    project_id: Option<&str>,
) -> Vec<MemoryItem> {
    let half_life_days = 365.0_f32;
    let layer_weight = 3.0_f32;
    let mut scored: Vec<(f32, MemoryItem)> = hits
        .into_iter()
        .map(|hit| {
            let recency = scorer::recency_decay(now, hit.note.updated_at, half_life_days);
            let source_boost = match hit.note.source_type {
                NoteSourceType::UserExplicit => 1.0,
                NoteSourceType::AgentInferred => 0.6,
            };
            let scope_boost = match (hit.note.scope, hit.note.scope_ref.as_deref()) {
                // global notes already go through persistent injection,
                // so we down-weight them in recall to avoid duplicating
                // the same fact in two prompt blocks.
                (NoteScope::Global, _) => 0.3,
                (NoteScope::Project, Some(pid)) if Some(pid) == project_id => 1.5,
                (NoteScope::Project, _) => 1.0,
                (NoteScope::Session, _) => 1.0,
            };
            let score = hit.score.max(0.001) * layer_weight * recency * source_boost * scope_boost;
            let item = MemoryItem::Note {
                id: hit.note.id,
                content: hit.note.content,
                score,
                scope: hit.note.scope,
                source_type: hit.note.source_type,
                status: hit.note.status,
            };
            (score, item)
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    pack_within_budget(scored, budget_chars, |item| match item {
        MemoryItem::Note { content, .. } => content.chars().count() + 30,
        _ => 0,
    })
}

fn score_and_pack_messages(
    hits: Vec<MessageHit>,
    now: DateTime<Utc>,
    budget_chars: usize,
    current_thread_id: Option<&str>,
) -> Vec<MemoryItem> {
    let half_life_days = 30.0_f32;
    let layer_weight = 0.5_f32;
    let mut scored: Vec<(f32, MemoryItem)> = hits
        .into_iter()
        // Exclude messages from the current thread — those are already
        // in the live conversation context.
        .filter(|hit| {
            current_thread_id
                .map(|tid| hit.message.thread_id != tid)
                .unwrap_or(true)
        })
        .map(|hit| {
            let recency = scorer::recency_decay(now, hit.message.created_at, half_life_days);
            let score = hit.score.max(0.001) * layer_weight * recency;
            let text = hit.message.content_text;
            let excerpt: String = text.chars().take(220).collect();
            let item = MemoryItem::Message {
                id: hit.message.id,
                thread_id: hit.message.thread_id,
                content: excerpt,
                score,
                ts: hit.message.created_at,
            };
            (score, item)
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    pack_within_budget(scored, budget_chars, |item| match item {
        MemoryItem::Message { content, .. } => content.chars().count() + 60,
        _ => 0,
    })
}

fn pack_within_budget(
    scored: Vec<(f32, MemoryItem)>,
    budget_chars: usize,
    cost_of: impl Fn(&MemoryItem) -> usize,
) -> Vec<MemoryItem> {
    let mut out: Vec<MemoryItem> = Vec::new();
    let mut budget = budget_chars;
    for (_, item) in scored {
        let cost = cost_of(&item);
        if cost > budget && !out.is_empty() {
            break;
        }
        budget = budget.saturating_sub(cost);
        out.push(item);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::test_in_memory_pool;
    use crate::db::repos::chat::{
        ChatMessageRepo, ChatThreadRepo, NewChatThread, NewUserMessage, SqliteChatMessageRepo,
        SqliteChatThreadRepo,
    };
    use crate::db::repos::frames::SqliteFrameRepo;
    use crate::db::repos::notes::{NewNote, SqliteNotesRepo};
    use crate::domain::chat::ChatThreadKind;
    use crate::domain::config::ApiShape;
    use crate::domain::note::{NoteScope, NoteSourceType};

    const SCHEMA_SQL: &str = include_str!("../../db/schema.sql");

    async fn build_service() -> (
        MemoryService,
        std::sync::Arc<SqliteChatThreadRepo>,
        std::sync::Arc<SqliteChatMessageRepo>,
        std::sync::Arc<SqliteNotesRepo>,
    ) {
        let pool = test_in_memory_pool().unwrap();
        pool.get().unwrap().execute_batch(SCHEMA_SQL).unwrap();
        let frames = std::sync::Arc::new(SqliteFrameRepo::new(pool.clone()));
        let notes = std::sync::Arc::new(SqliteNotesRepo::new(pool.clone()));
        let messages = std::sync::Arc::new(SqliteChatMessageRepo::new(pool.clone()));
        let threads = std::sync::Arc::new(SqliteChatThreadRepo::new(pool));
        let service = MemoryService::new(
            frames as std::sync::Arc<dyn crate::db::repos::frames::FrameRepo>,
            notes.clone() as std::sync::Arc<dyn crate::db::repos::notes::NotesRepo>,
            messages.clone() as std::sync::Arc<dyn ChatMessageRepo>,
        );
        (service, threads, messages, notes)
    }

    #[tokio::test]
    async fn recall_returns_note_for_matching_query() {
        let (service, _t, _m, notes) = build_service().await;
        notes
            .create(NewNote {
                content: "用 4 空格缩进 Python".into(),
                scope: NoteScope::Session,
                scope_ref: None,
                source_type: NoteSourceType::UserExplicit,
                source_message_id: None,
                source_thread_id: None,
                confidence: None,
                status: None,
                expires_at: None,
            })
            .await
            .unwrap();
        let req = RecallRequest::new("我现在写 Python，记不清缩进规则".into());
        let result = service.recall(&req).await.unwrap();
        let has_note = result
            .items
            .iter()
            .any(|i| matches!(i, MemoryItem::Note { content, .. } if content.contains("Python")));
        assert!(has_note, "expected the Python indent note in recall");
    }

    #[tokio::test]
    async fn recall_excludes_messages_from_current_thread() {
        let (service, threads, messages, _notes) = build_service().await;
        let t = threads
            .create(NewChatThread {
                title: Some("debug".into()),
                bound_model_id: "claude-sonnet-4-6".into(),
                bound_api_shape: ApiShape::Anthropic,
                kind: ChatThreadKind::User,
                system_task: None,
            })
            .await
            .unwrap();
        messages
            .insert_user_message(NewUserMessage {
                thread_id: t.id.clone(),
                text: "useReducer 替代 useState".into(),
                focus_context: None,
            })
            .await
            .unwrap();
        let mut req = RecallRequest::new("useReducer".into());
        req.thread_id = Some(t.id.clone());
        let result = service.recall(&req).await.unwrap();
        let same_thread_hit = result
            .items
            .iter()
            .any(|i| matches!(i, MemoryItem::Message { thread_id, .. } if thread_id == &t.id));
        assert!(
            !same_thread_hit,
            "messages from the current thread should not surface in recall"
        );
    }

    #[tokio::test]
    async fn empty_query_short_circuits_to_no_hits() {
        let (service, _t, _m, _n) = build_service().await;
        let req = RecallRequest::new("   ,. !".into());
        let result = service.recall(&req).await.unwrap();
        assert!(result.items.is_empty());
    }
}
