//! `/ask` chat repos (spec §五).
//!
//! Two tables behind one trait per resource: `ChatThreadRepo` for the
//! thread containers, `ChatMessageRepo` for the messages within them.
//! Threads cascade-delete their messages via FK.
//!
//! v1200 lifecycle (vs. legacy `append`-once-per-turn):
//!
//! * `insert_user_message` — fires the moment the user hits send. The
//!   row is durable from this point on regardless of what happens to
//!   the assistant stream.
//! * `start_assistant_message` — drops a `status='streaming'` placeholder
//!   so the row owns a stable `id` + `created_at` before the agent
//!   even spawns. The `by_thread` reader filters these out so the live
//!   UI only sees finalized turns; the only consumer of streaming rows
//!   is the boot-time orphan sweep.
//! * `finalize_assistant_message` — the streaming row's terminal write.
//!   Status moves to `complete` / `error` / `cancelled` and the
//!   accumulated `content_blocks` JSON is committed.
//! * `cancel_streaming_orphans` — boot hook. Any `status='streaming'`
//!   row left over from a previous app crash is flipped to
//!   `cancelled`. The user sees "上次回答没跑完" instead of a row
//!   stuck in a perpetually-spinning state.

use async_trait::async_trait;
use rusqlite::{params, OptionalExtension};
use ulid::Ulid;

use crate::{
    db::{
        pool::{run_blocking, DbPool},
        time::DbInstant,
    },
    domain::{
        chat::{
            ChatMessage, ChatRole, ChatThread, ChatThreadKind, ContentBlock, MessageStatus,
            SystemTaskKind, Usage,
        },
        config::ApiShape,
    },
    error::{CorivoError, Result},
};

// ---------------------------------------------------------------------------
// Threads
// ---------------------------------------------------------------------------

/// Inputs for creating a new chat thread (spec §8.2).
#[derive(Debug, Clone)]
pub struct NewChatThread {
    pub title: Option<String>,
    /// Permanent binding — never mutated after creation.
    pub bound_model_id: String,
    pub bound_api_shape: ApiShape,
    /// memory-system-spec §11.2. Defaults to `User`; background agent
    /// tasks pass `System` + a `system_task` value.
    pub kind: ChatThreadKind,
    pub system_task: Option<SystemTaskKind>,
}

impl Default for NewChatThread {
    fn default() -> Self {
        Self {
            title: None,
            bound_model_id: String::new(),
            bound_api_shape: ApiShape::Anthropic,
            kind: ChatThreadKind::User,
            system_task: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ThreadSummaryUpdate {
    pub thread_id: String,
    pub summary: String,
    pub summary_topics: String,
}

#[derive(Debug, Clone)]
pub struct ThreadSummaryHit {
    pub thread_id: String,
    pub title: Option<String>,
    pub summary: String,
    pub summary_topics: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub summary_updated_at: chrono::DateTime<chrono::Utc>,
    pub score: f32,
}

#[async_trait]
pub trait ChatThreadRepo: Send + Sync {
    async fn create(&self, new: NewChatThread) -> Result<ChatThread>;
    /// Returns only `kind='user'` threads. Background agent task threads
    /// (memory-system-spec §11.2) never enter the sidebar.
    async fn list(&self, limit: u32) -> Result<Vec<ChatThread>>;
    /// Returns only `kind='system'` threads, optionally filtered by
    /// `system_task`. Drives the Settings 诊断 panel.
    async fn list_system(
        &self,
        task: Option<SystemTaskKind>,
        limit: u32,
    ) -> Result<Vec<ChatThread>>;
    async fn by_id(&self, id: &str) -> Result<Option<ChatThread>>;
    async fn rename(&self, id: &str, title: Option<String>) -> Result<()>;
    async fn touch(&self, id: &str) -> Result<()>;
    async fn delete(&self, id: &str) -> Result<()>;
    /// v1300 sidebar lifecycle. `Some(now)` pins, `None` un-pins.
    /// `updated_at` is left alone — pinning is a metadata change, not
    /// fresh activity, and would otherwise leak a stale "just touched"
    /// signal into the recent list.
    async fn set_pinned(&self, id: &str, pinned: bool) -> Result<()>;
    /// v1300 sidebar lifecycle. `Some(now)` archives, `None` unarchives.
    /// Same `updated_at` discipline as `set_pinned`.
    async fn set_archived(&self, id: &str, archived: bool) -> Result<()>;
    /// memory-system-spec §12.3. Session learner overwrites the entire
    /// summary + topics per run (cheap; threads are short). `topics`
    /// should be the Rust-tokenized space-separated form.
    async fn update_summary(&self, update: ThreadSummaryUpdate) -> Result<()>;
    /// memory-system-spec §12.4.1. FTS5 search over `thread_summaries_fts`
    /// (only `kind='user'` rows participate, see schema triggers).
    /// Returns hits ordered by BM25 score with a 60-day recency
    /// half-life applied.
    async fn search_summaries(&self, query: &str, limit: u32) -> Result<Vec<ThreadSummaryHit>>;
    /// Latest message id in `thread_id` (any role). Used by the session
    /// learner to advance its checkpoint without re-reading the whole
    /// thread. Returns `None` if the thread has no messages.
    async fn latest_message_id(&self, thread_id: &str) -> Result<Option<String>>;
}

#[derive(Clone)]
pub struct SqliteChatThreadRepo {
    pool: DbPool,
}

impl SqliteChatThreadRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
}

/// Pre-tokenize `content_text` for the chat_messages_fts index.
/// Empty input maps to an empty string (we still NULL the column at
/// write-time to keep the trigger from indexing blank rows).
fn derive_chat_search_tokens(content_text: &str) -> Option<String> {
    let trimmed = content_text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let tokens = crate::services::tokenize::tokenize_for_index(trimmed);
    if tokens.is_empty() {
        None
    } else {
        Some(tokens)
    }
}

const THREAD_COLUMNS: &str = "id, title, bound_model_id, bound_api_shape, pinned_at, archived_at, \
     kind, system_task, summary, summary_topics, summary_updated_at, \
     created_at, updated_at";

#[async_trait]
impl ChatThreadRepo for SqliteChatThreadRepo {
    async fn create(&self, new: NewChatThread) -> Result<ChatThread> {
        let id = Ulid::new().to_string();
        let now = DbInstant::now();
        let return_id = id.clone();
        let title = new.title;
        let bound_model_id = new.bound_model_id;
        let bound_api_shape = new.bound_api_shape.as_str().to_string();
        let kind = new.kind.as_str().to_string();
        let system_task = new.system_task.map(|t| t.as_str().to_string());
        run_blocking(self.pool.clone(), move |conn| {
            conn.execute(
                "INSERT INTO chat_threads
                   (id, title, bound_model_id, bound_api_shape,
                    pinned_at, archived_at, kind, system_task,
                    created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, NULL, NULL, ?5, ?6, ?7, ?7)",
                params![
                    id,
                    title,
                    bound_model_id,
                    bound_api_shape,
                    kind,
                    system_task,
                    now
                ],
            )
            .map_err(|error| CorivoError::Internal(format!("插入 chat_thread 失败: {error}")))?;
            let sql = format!("SELECT {THREAD_COLUMNS} FROM chat_threads WHERE id = ?1");
            conn.query_row(&sql, params![return_id], row_to_thread)
                .map_err(|error| CorivoError::Internal(format!("回读 chat_thread 失败: {error}")))
        })
        .await
    }

    /// Returns the most recently-touched threads, regardless of title.
    ///
    /// Quick Ask and `/ask` both append to `chat_threads`; we no longer
    /// distinguish "anonymous scratch" from "promoted" threads at the
    /// repo layer. A thread with no messages yet (e.g. just-created via
    /// type-to-create on `/ask`, or a fresh Quick Ask invoke that the
    /// user closed without sending) still has its `created_at` /
    /// `updated_at` row, but `chat_user_message_create` won't have run,
    /// so the title stays NULL and the sidebar uses `updated_at` as the
    /// label fallback ([thread-list.tsx](../../../../src/pages/ask/thread-list.tsx)).
    async fn list(&self, limit: u32) -> Result<Vec<ChatThread>> {
        let limit = limit.max(1) as i64;
        run_blocking(self.pool.clone(), move |conn| {
            let sql = format!(
                "SELECT {THREAD_COLUMNS} FROM chat_threads
                 WHERE kind = 'user'
                 ORDER BY updated_at DESC LIMIT ?1"
            );
            let mut stmt = conn.prepare(&sql).map_err(|error| {
                CorivoError::Internal(format!("prepare chat_threads list 失败: {error}"))
            })?;
            let rows = stmt
                .query_map(params![limit], row_to_thread)
                .map_err(|error| {
                    CorivoError::Internal(format!("query chat_threads list 失败: {error}"))
                })?;
            rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|error| {
                CorivoError::Internal(format!("collect chat_threads list 失败: {error}"))
            })
        })
        .await
    }

    async fn list_system(
        &self,
        task: Option<SystemTaskKind>,
        limit: u32,
    ) -> Result<Vec<ChatThread>> {
        let limit = limit.max(1) as i64;
        let task_filter = task.map(|t| t.as_str().to_string());
        run_blocking(self.pool.clone(), move |conn| {
            let sql = format!(
                "SELECT {THREAD_COLUMNS} FROM chat_threads
                 WHERE kind = 'system'
                   AND (?1 IS NULL OR system_task = ?1)
                 ORDER BY created_at DESC LIMIT ?2"
            );
            let mut stmt = conn.prepare(&sql).map_err(|error| {
                CorivoError::Internal(format!("prepare chat_threads list_system 失败: {error}"))
            })?;
            let rows = stmt
                .query_map(params![task_filter, limit], row_to_thread)
                .map_err(|error| {
                    CorivoError::Internal(format!("query chat_threads list_system 失败: {error}"))
                })?;
            rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|error| {
                CorivoError::Internal(format!("collect chat_threads list_system 失败: {error}"))
            })
        })
        .await
    }

    async fn update_summary(&self, update: ThreadSummaryUpdate) -> Result<()> {
        let now = DbInstant::now();
        run_blocking(self.pool.clone(), move |conn| {
            let updated = conn
                .execute(
                    "UPDATE chat_threads
                       SET summary = ?1,
                           summary_topics = ?2,
                           summary_updated_at = ?3
                     WHERE id = ?4",
                    params![update.summary, update.summary_topics, now, update.thread_id],
                )
                .map_err(|error| {
                    CorivoError::Internal(format!("UPDATE thread summary 失败: {error}"))
                })?;
            if updated == 0 {
                return Err(CorivoError::Internal(format!(
                    "thread {} 不存在",
                    update.thread_id
                )));
            }
            Ok(())
        })
        .await
    }

    async fn search_summaries(&self, query: &str, limit: u32) -> Result<Vec<ThreadSummaryHit>> {
        let query = query.to_string();
        let limit = limit.max(1).min(50) as i64;
        run_blocking(self.pool.clone(), move |conn| {
            // 60 day half-life recency adjustment baked into the ORDER BY.
            // BM25 in SQLite FTS5 returns a *lower* number for a better
            // match (it's negated), so we flip the sign before applying
            // the decay multiplier.
            let sql = format!(
                "SELECT t.id, t.title, t.summary, t.summary_topics,
                        t.created_at,
                        COALESCE(t.summary_updated_at, t.created_at) AS sum_at,
                        -bm25(thread_summaries_fts) AS raw_score
                 FROM thread_summaries_fts
                 JOIN chat_threads t ON t.rowid = thread_summaries_fts.rowid
                 WHERE thread_summaries_fts MATCH ?1
                   AND t.kind = 'user'
                 ORDER BY raw_score DESC
                 LIMIT ?2"
            );
            let mut stmt = conn.prepare(&sql).map_err(|error| {
                CorivoError::Internal(format!("prepare thread_summaries_fts 失败: {error}"))
            })?;
            let rows = stmt
                .query_map(params![query, limit], |row| {
                    let summary_topics: Option<String> = row.get(3)?;
                    let created_at: DbInstant = row.get(4)?;
                    let summary_updated_at: DbInstant = row.get(5)?;
                    let raw_score: f64 = row.get(6)?;
                    Ok(ThreadSummaryHit {
                        thread_id: row.get(0)?,
                        title: row.get(1)?,
                        summary: row.get(2)?,
                        summary_topics,
                        created_at: created_at.into_inner(),
                        summary_updated_at: summary_updated_at.into_inner(),
                        score: raw_score as f32,
                    })
                })
                .map_err(|error| {
                    CorivoError::Internal(format!("query thread_summaries_fts 失败: {error}"))
                })?;
            let mut hits = rows
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|error| {
                    CorivoError::Internal(format!("collect thread_summaries_fts 失败: {error}"))
                })?;
            // 60-day half-life recency multiplier; applied in Rust so we
            // can use the live Clock rather than SQLite's `julianday`
            // (which we'd have to ban under time discipline anyway).
            let now = crate::db::time::now_utc();
            let half_life_days = 60.0_f32;
            for hit in &mut hits {
                let age_days =
                    (now - hit.summary_updated_at).num_seconds().max(0) as f32 / 86_400.0;
                let decay = (-age_days / half_life_days * std::f32::consts::LN_2).exp();
                hit.score *= decay;
            }
            hits.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            Ok(hits)
        })
        .await
    }

    async fn latest_message_id(&self, thread_id: &str) -> Result<Option<String>> {
        let thread_id = thread_id.to_string();
        run_blocking(self.pool.clone(), move |conn| {
            conn.query_row(
                "SELECT id FROM chat_messages
                 WHERE thread_id = ?1
                 ORDER BY created_at DESC, id DESC
                 LIMIT 1",
                params![thread_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| CorivoError::Internal(format!("latest_message_id 失败: {error}")))
        })
        .await
    }

    async fn by_id(&self, id: &str) -> Result<Option<ChatThread>> {
        let id = id.to_string();
        run_blocking(self.pool.clone(), move |conn| {
            let sql = format!("SELECT {THREAD_COLUMNS} FROM chat_threads WHERE id = ?1");
            conn.query_row(&sql, params![id], row_to_thread)
                .optional()
                .map_err(|error| CorivoError::Internal(format!("查询 chat_thread 失败: {error}")))
        })
        .await
    }

    async fn rename(&self, id: &str, title: Option<String>) -> Result<()> {
        let id = id.to_string();
        let now = DbInstant::now();
        run_blocking(self.pool.clone(), move |conn| {
            conn.execute(
                "UPDATE chat_threads SET title = ?1, updated_at = ?2 WHERE id = ?3",
                params![title, now, id],
            )
            .map_err(|error| CorivoError::Internal(format!("UPDATE chat_thread 失败: {error}")))?;
            Ok(())
        })
        .await
    }

    async fn touch(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        let now = DbInstant::now();
        run_blocking(self.pool.clone(), move |conn| {
            conn.execute(
                "UPDATE chat_threads SET updated_at = ?1 WHERE id = ?2",
                params![now, id],
            )
            .map_err(|error| CorivoError::Internal(format!("touch chat_thread 失败: {error}")))?;
            Ok(())
        })
        .await
    }

    async fn delete(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        run_blocking(self.pool.clone(), move |conn| {
            conn.execute("DELETE FROM chat_threads WHERE id = ?1", params![id])
                .map_err(|error| {
                    CorivoError::Internal(format!("DELETE chat_thread 失败: {error}"))
                })?;
            Ok(())
        })
        .await
    }

    async fn set_pinned(&self, id: &str, pinned: bool) -> Result<()> {
        let id = id.to_string();
        let stamp = if pinned { Some(DbInstant::now()) } else { None };
        run_blocking(self.pool.clone(), move |conn| {
            conn.execute(
                "UPDATE chat_threads SET pinned_at = ?1 WHERE id = ?2",
                params![stamp, id],
            )
            .map_err(|error| CorivoError::Internal(format!("set pinned_at 失败: {error}")))?;
            Ok(())
        })
        .await
    }

    async fn set_archived(&self, id: &str, archived: bool) -> Result<()> {
        let id = id.to_string();
        let stamp = if archived {
            Some(DbInstant::now())
        } else {
            None
        };
        run_blocking(self.pool.clone(), move |conn| {
            conn.execute(
                "UPDATE chat_threads SET archived_at = ?1 WHERE id = ?2",
                params![stamp, id],
            )
            .map_err(|error| CorivoError::Internal(format!("set archived_at 失败: {error}")))?;
            Ok(())
        })
        .await
    }
}

fn row_to_thread(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatThread> {
    let api_shape_raw: String = row.get(3)?;
    let bound_api_shape = parse_api_shape(&api_shape_raw).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown bound_api_shape: {api_shape_raw}"),
            )),
        )
    })?;
    let kind_raw: String = row.get(6)?;
    let kind = ChatThreadKind::parse(&kind_raw).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            6,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown thread kind: {kind_raw}"),
            )),
        )
    })?;
    let system_task_raw: Option<String> = row.get(7)?;
    let system_task = system_task_raw.as_deref().and_then(SystemTaskKind::parse);
    Ok(ChatThread {
        id: row.get(0)?,
        title: row.get(1)?,
        bound_model_id: row.get(2)?,
        bound_api_shape,
        pinned_at: row
            .get::<_, Option<DbInstant>>(4)?
            .map(DbInstant::into_inner),
        archived_at: row
            .get::<_, Option<DbInstant>>(5)?
            .map(DbInstant::into_inner),
        kind,
        system_task,
        summary: row.get(8)?,
        summary_topics: row.get(9)?,
        summary_updated_at: row
            .get::<_, Option<DbInstant>>(10)?
            .map(DbInstant::into_inner),
        created_at: row.get::<_, DbInstant>(11)?.into_inner(),
        updated_at: row.get::<_, DbInstant>(12)?.into_inner(),
    })
}

fn parse_api_shape(raw: &str) -> Option<ApiShape> {
    match raw {
        "anthropic" => Some(ApiShape::Anthropic),
        "openai" => Some(ApiShape::Openai),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// Inputs for creating a user message. Persists immediately on send —
/// the user's prompt is durable before the agent stream even starts.
#[derive(Debug, Clone)]
pub struct NewUserMessage {
    pub thread_id: String,
    /// Plain text the user typed. The repo wraps this into a single
    /// `ContentBlock::Text` and prepends a [`ContentBlock::FocusContext`]
    /// when `focus_context` is `Some` (Quick Ask path).
    pub text: String,
    pub focus_context: Option<UserFocusContext>,
}

/// Mirror of [`crate::domain::chat::ContentBlock::FocusContext`] without
/// the enum wrapper, so the command layer can build it without
/// importing the enum directly.
#[derive(Debug, Clone)]
pub struct UserFocusContext {
    pub frame_id: String,
    pub summary: String,
    pub primary_text: Option<String>,
    pub selection: Option<String>,
}

/// Final assistant payload — the result of accumulating the sidecar
/// stream into a single canonical content-blocks array. Passed to
/// [`ChatMessageRepo::finalize_assistant_message`] when the stream
/// terminates (success / error / cancel).
#[derive(Debug, Clone)]
pub struct FinalizeAssistant {
    pub message_id: String,
    pub content_blocks: Vec<ContentBlock>,
    pub cited_frame_ids: Vec<String>,
    pub status: MessageStatus,
    pub error_message: Option<String>,
    pub finish_reason: Option<String>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone)]
pub struct MessageHit {
    pub message: ChatMessage,
    /// Raw BM25 score (already negated so higher is better; the recall
    /// layer multiplies recency / scope boosts on top).
    pub score: f32,
}

#[async_trait]
pub trait ChatMessageRepo: Send + Sync {
    /// Insert a `status='complete'` user row immediately. Returns the
    /// canonical row (with id + created_at) so the caller can emit
    /// updates / pass the id around.
    async fn insert_user_message(&self, new: NewUserMessage) -> Result<ChatMessage>;

    /// Insert a `status='streaming'` assistant placeholder. The
    /// stream consumer accumulates blocks in memory and calls
    /// `finalize_assistant_message` to flip status + commit blocks.
    async fn start_assistant_message(&self, thread_id: &str) -> Result<ChatMessage>;

    /// Stamp `model_used` on an existing assistant row. Called by
    /// `exec_agent_send` right after `resolve_corivo_runtime` decides
    /// which alias drives the turn — happens before the sidecar
    /// streams its first byte, so the audit lands even on a crash.
    /// Idempotent: re-running with the same alias is a no-op.
    async fn set_assistant_model_used(&self, message_id: &str, alias: &str) -> Result<()>;

    /// Terminal write for an assistant turn. Status moves to one of
    /// `complete` / `error` / `cancelled`; `content_blocks` /
    /// `usage` / `finish_reason` are committed; `updated_at` set to now.
    async fn finalize_assistant_message(&self, payload: FinalizeAssistant) -> Result<ChatMessage>;

    /// Boot-time cleanup. Anything still `status='streaming'` is left
    /// from a crash / kill — flip it to `cancelled` so the UI doesn't
    /// render a perpetually-spinning row. Returns the number of rows
    /// touched for logging.
    async fn cancel_streaming_orphans(&self) -> Result<u64>;

    /// Read finalized messages for `thread_id`. Streaming placeholders
    /// are filtered out — only the boot-time orphan sweep ever sees them.
    async fn by_thread(&self, thread_id: &str) -> Result<Vec<ChatMessage>>;

    /// FTS5 over chat_messages_fts. Always joined against chat_threads
    /// so we can scope to `kind='user'` only — background agent task
    /// turns never participate in recall.
    async fn fts_search(&self, fts_query: &str, limit: u32) -> Result<Vec<MessageHit>>;
}

#[derive(Clone)]
pub struct SqliteChatMessageRepo {
    pool: DbPool,
}

impl SqliteChatMessageRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
}

const MESSAGE_COLUMNS: &str = "id, thread_id, role, content_blocks, content_text, \
     cited_frame_ids, status, error_message, finish_reason, usage, created_at, \
     updated_at, model_used";

#[async_trait]
impl ChatMessageRepo for SqliteChatMessageRepo {
    async fn insert_user_message(&self, new: NewUserMessage) -> Result<ChatMessage> {
        let id = Ulid::new().to_string();
        let now = DbInstant::now();
        let return_id = id.clone();

        // Build the user-side content_blocks: optional FocusContext
        // first, then the plain Text. Quick Ask attaches the focus
        // block; `/ask` doesn't.
        let mut blocks: Vec<ContentBlock> = Vec::new();
        let mut cited: Vec<String> = Vec::new();
        if let Some(fc) = new.focus_context {
            cited.push(fc.frame_id.clone());
            blocks.push(ContentBlock::FocusContext {
                frame_id: fc.frame_id,
                summary: fc.summary,
                primary_text: fc.primary_text,
                selection: fc.selection,
            });
        }
        blocks.push(ContentBlock::Text {
            text: new.text.clone(),
        });

        let blocks_json = serde_json::to_string(&blocks).map_err(|e| {
            CorivoError::Internal(format!("serialize user content_blocks failed: {e}"))
        })?;
        let cited_json = if cited.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&cited).map_err(|e| {
                CorivoError::Internal(format!("serialize cited_frame_ids failed: {e}"))
            })?)
        };
        let content_text = new.text;
        let thread_id = new.thread_id.clone();

        let search_tokens: Option<String> = derive_chat_search_tokens(&content_text);
        run_blocking(self.pool.clone(), move |conn| {
            conn.execute(
                "INSERT INTO chat_messages
                   (id, thread_id, role, content_blocks, content_text,
                    cited_frame_ids, status, search_tokens, created_at)
                 VALUES (?1, ?2, 'user', ?3, ?4, ?5, 'complete', ?6, ?7)",
                params![
                    id,
                    thread_id,
                    blocks_json,
                    content_text,
                    cited_json,
                    search_tokens,
                    now
                ],
            )
            .map_err(|error| {
                CorivoError::Internal(format!("插入 chat_message (user) 失败: {error}"))
            })?;
            conn.execute(
                "UPDATE chat_threads SET updated_at = ?1 WHERE id = ?2",
                params![now, thread_id],
            )
            .map_err(|error| CorivoError::Internal(format!("touch chat_thread 失败: {error}")))?;
            let sql = format!("SELECT {MESSAGE_COLUMNS} FROM chat_messages WHERE id = ?1");
            conn.query_row(&sql, params![return_id], row_to_message)
                .map_err(|error| {
                    CorivoError::Internal(format!("回读 chat_message (user) 失败: {error}"))
                })
        })
        .await
    }

    async fn start_assistant_message(&self, thread_id: &str) -> Result<ChatMessage> {
        let id = Ulid::new().to_string();
        let now = DbInstant::now();
        let return_id = id.clone();
        let thread_id = thread_id.to_string();
        run_blocking(self.pool.clone(), move |conn| {
            conn.execute(
                "INSERT INTO chat_messages
                   (id, thread_id, role, content_blocks, content_text,
                    status, created_at)
                 VALUES (?1, ?2, 'assistant', '[]', '', 'streaming', ?3)",
                params![id, thread_id, now],
            )
            .map_err(|error| {
                CorivoError::Internal(format!(
                    "插入 chat_message (assistant placeholder) 失败: {error}"
                ))
            })?;
            // Don't bump chat_threads.updated_at here — the user row
            // already touched it in the same turn, and we don't want
            // streaming placeholders to ALSO move the sidebar.
            let sql = format!("SELECT {MESSAGE_COLUMNS} FROM chat_messages WHERE id = ?1");
            conn.query_row(&sql, params![return_id], row_to_message)
                .map_err(|error| {
                    CorivoError::Internal(format!(
                        "回读 chat_message (assistant placeholder) 失败: {error}"
                    ))
                })
        })
        .await
    }

    async fn set_assistant_model_used(&self, message_id: &str, alias: &str) -> Result<()> {
        let id = message_id.to_string();
        let alias = alias.to_string();
        run_blocking(self.pool.clone(), move |conn| {
            conn.execute(
                "UPDATE chat_messages
                   SET model_used = ?1
                 WHERE id = ?2 AND role = 'assistant'",
                params![alias, id],
            )
            .map_err(|error| {
                CorivoError::Internal(format!("UPDATE chat_message.model_used 失败: {error}"))
            })?;
            // No affected-row check — exec_agent_send may race the
            // stream-start path, and the audit is best-effort: a
            // missed UPDATE just leaves model_used NULL, which the
            // UI already handles as "unknown model".
            Ok(())
        })
        .await
    }

    async fn finalize_assistant_message(&self, payload: FinalizeAssistant) -> Result<ChatMessage> {
        let now = DbInstant::now();
        let blocks_json = serde_json::to_string(&payload.content_blocks)
            .map_err(|e| CorivoError::Internal(format!("serialize content_blocks failed: {e}")))?;
        let cited_json = if payload.cited_frame_ids.is_empty() {
            None
        } else {
            Some(
                serde_json::to_string(&payload.cited_frame_ids).map_err(|e| {
                    CorivoError::Internal(format!("serialize cited_frame_ids failed: {e}"))
                })?,
            )
        };
        let usage_json = match &payload.usage {
            Some(u) => Some(
                serde_json::to_string(u)
                    .map_err(|e| CorivoError::Internal(format!("serialize usage failed: {e}")))?,
            ),
            None => None,
        };
        let content_text = blocks_to_text(&payload.content_blocks);
        let status_str = payload.status.as_str().to_string();
        let id = payload.message_id.clone();
        let return_id = id.clone();
        let error_message = payload.error_message;
        let finish_reason = payload.finish_reason;

        run_blocking(self.pool.clone(), move |conn| {
            // Pull thread_id off the streaming row so we can also touch
            // chat_threads in the same transaction.
            let thread_id: String = conn
                .query_row(
                    "SELECT thread_id FROM chat_messages WHERE id = ?1",
                    params![id],
                    |r| r.get(0),
                )
                .map_err(|error| {
                    CorivoError::Internal(format!(
                        "finalize: chat_message {id} 不存在或读取失败: {error}"
                    ))
                })?;

            // Index assistant turns once they're complete so the
            // recall layer can find them. Tokenizing on error / cancel
            // is wasted work — leave search_tokens NULL on those.
            let search_tokens_param: Option<String> = if status_str == "complete" {
                derive_chat_search_tokens(&content_text)
            } else {
                None
            };
            let updated = conn
                .execute(
                    "UPDATE chat_messages
                       SET content_blocks = ?1,
                           content_text = ?2,
                           cited_frame_ids = ?3,
                           status = ?4,
                           error_message = ?5,
                           finish_reason = ?6,
                           usage = ?7,
                           search_tokens = ?8,
                           updated_at = ?9
                     WHERE id = ?10",
                    params![
                        blocks_json,
                        content_text,
                        cited_json,
                        status_str,
                        error_message,
                        finish_reason,
                        usage_json,
                        search_tokens_param,
                        now,
                        id,
                    ],
                )
                .map_err(|error| {
                    CorivoError::Internal(format!("UPDATE chat_message (finalize) 失败: {error}"))
                })?;
            if updated == 0 {
                return Err(CorivoError::Internal(format!(
                    "finalize: chat_message {id} 没有任何行被更新"
                )));
            }
            conn.execute(
                "UPDATE chat_threads SET updated_at = ?1 WHERE id = ?2",
                params![now, thread_id],
            )
            .map_err(|error| {
                CorivoError::Internal(format!("touch chat_thread (finalize) 失败: {error}"))
            })?;

            let sql = format!("SELECT {MESSAGE_COLUMNS} FROM chat_messages WHERE id = ?1");
            conn.query_row(&sql, params![return_id], row_to_message)
                .map_err(|error| {
                    CorivoError::Internal(format!("回读 chat_message (finalize) 失败: {error}"))
                })
        })
        .await
    }

    async fn cancel_streaming_orphans(&self) -> Result<u64> {
        let now = DbInstant::now();
        run_blocking(self.pool.clone(), move |conn| {
            let updated = conn
                .execute(
                    "UPDATE chat_messages
                       SET status = 'cancelled',
                           error_message = COALESCE(error_message, '上次启动时这一轮回复未跑完'),
                           updated_at = ?1
                     WHERE status = 'streaming'",
                    params![now],
                )
                .map_err(|error| {
                    CorivoError::Internal(format!("cancel_streaming_orphans 更新失败: {error}"))
                })?;
            Ok(updated as u64)
        })
        .await
    }

    async fn fts_search(&self, fts_query: &str, limit: u32) -> Result<Vec<MessageHit>> {
        let q = fts_query.to_string();
        let limit = limit.max(1).min(100) as i64;
        run_blocking(self.pool.clone(), move |conn| {
            let qualified = MESSAGE_COLUMNS
                .split(',')
                .map(|c| format!("chat_messages.{}", c.trim()))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT {qualified}, -bm25(chat_messages_fts) AS score
                 FROM chat_messages_fts
                 JOIN chat_messages ON chat_messages.rowid = chat_messages_fts.rowid
                 JOIN chat_threads ON chat_threads.id = chat_messages.thread_id
                 WHERE chat_messages_fts MATCH ?1
                   AND chat_threads.kind = 'user'
                   AND chat_messages.status = 'complete'
                 ORDER BY score DESC
                 LIMIT ?2"
            );
            let mut stmt = conn.prepare(&sql).map_err(|error| {
                CorivoError::Internal(format!("prepare chat_messages_fts 失败: {error}"))
            })?;
            let rows = stmt
                .query_map(params![q, limit], |row| {
                    let message = row_to_message(row)?;
                    let score: f64 = row.get(13)?;
                    Ok(MessageHit {
                        message,
                        score: score as f32,
                    })
                })
                .map_err(|error| {
                    CorivoError::Internal(format!("query chat_messages_fts 失败: {error}"))
                })?;
            rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|error| {
                CorivoError::Internal(format!("collect chat_messages_fts 失败: {error}"))
            })
        })
        .await
    }

    async fn by_thread(&self, thread_id: &str) -> Result<Vec<ChatMessage>> {
        let thread_id = thread_id.to_string();
        run_blocking(self.pool.clone(), move |conn| {
            // ORDER BY note: inserts within the same millisecond would
            // tie on `created_at` (DbInstant is millisecond-resolution).
            // The role-based CASE forces user → assistant order for
            // any tie; `id ASC` is the final fallback. Streaming
            // placeholders are filtered out — only the orphan sweep
            // sees them.
            let sql = format!(
                "SELECT {MESSAGE_COLUMNS} FROM chat_messages
                 WHERE thread_id = ?1 AND status != 'streaming'
                 ORDER BY created_at ASC,
                          CASE role
                              WHEN 'user' THEN 0
                              WHEN 'assistant' THEN 1
                              ELSE 2
                          END ASC,
                          id ASC"
            );
            let mut stmt = conn.prepare(&sql).map_err(|error| {
                CorivoError::Internal(format!("prepare chat_messages list 失败: {error}"))
            })?;
            let rows = stmt
                .query_map(params![thread_id], row_to_message)
                .map_err(|error| {
                    CorivoError::Internal(format!("query chat_messages list 失败: {error}"))
                })?;
            rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|error| {
                CorivoError::Internal(format!("collect chat_messages list 失败: {error}"))
            })
        })
        .await
    }
}

fn row_to_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatMessage> {
    let role_str: String = row.get(2)?;
    let role = ChatRole::parse(&role_str).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown chat role: {role_str}"),
            )),
        )
    })?;
    let blocks_str: String = row.get(3)?;
    let content_blocks: Vec<ContentBlock> = serde_json::from_str(&blocks_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid content_blocks JSON: {e}"),
            )),
        )
    })?;
    let content_text: String = row.get(4)?;
    let cited_raw: Option<String> = row.get(5)?;
    let cited_frame_ids = cited_raw
        .as_deref()
        .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok());
    let status_str: String = row.get(6)?;
    let status = MessageStatus::parse(&status_str).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            6,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown message status: {status_str}"),
            )),
        )
    })?;
    let error_message: Option<String> = row.get(7)?;
    let finish_reason: Option<String> = row.get(8)?;
    let usage_raw: Option<String> = row.get(9)?;
    let usage = usage_raw
        .as_deref()
        .and_then(|s| serde_json::from_str::<Usage>(s).ok());
    let updated_at_raw: Option<DbInstant> = row.get(11)?;
    let model_used: Option<String> = row.get(12)?;
    Ok(ChatMessage {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        role,
        content_blocks,
        content_text,
        cited_frame_ids,
        status,
        error_message,
        finish_reason,
        usage,
        model_used,
        created_at: row.get::<_, DbInstant>(10)?.into_inner(),
        updated_at: updated_at_raw.map(DbInstant::into_inner),
    })
}

/// Concatenate every text + thinking block into a single plain-text
/// string. Used to populate `content_text` so FTS / sidebar previews
/// don't have to parse the JSON column.
fn blocks_to_text(blocks: &[ContentBlock]) -> String {
    let mut buf = String::new();
    for block in blocks {
        match block {
            ContentBlock::Text { text } | ContentBlock::Thinking { text, .. } => {
                if !buf.is_empty() {
                    buf.push('\n');
                }
                buf.push_str(text);
            }
            // Tool use / result / focus / citation don't contribute to
            // the rollup. Their args / contents are heterogeneous and
            // the rollup is for human-readable preview.
            _ => {}
        }
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::test_in_memory_pool;

    const SCHEMA_SQL: &str = include_str!("../schema.sql");

    fn init_pool() -> DbPool {
        let pool = test_in_memory_pool().unwrap();
        let conn = pool.get().unwrap();
        conn.execute_batch(SCHEMA_SQL).unwrap();
        pool
    }

    fn fake_thread(title: Option<&str>) -> NewChatThread {
        NewChatThread {
            title: title.map(|s| s.to_string()),
            bound_model_id: "claude-sonnet-4-6".to_string(),
            bound_api_shape: ApiShape::Anthropic,
            kind: ChatThreadKind::User,
            system_task: None,
        }
    }

    #[tokio::test]
    async fn insert_user_message_persists_immediately_and_touches_thread() {
        let pool = init_pool();
        let threads = SqliteChatThreadRepo::new(pool.clone());
        let messages = SqliteChatMessageRepo::new(pool);

        let t = threads.create(fake_thread(None)).await.unwrap();
        let user = messages
            .insert_user_message(NewUserMessage {
                thread_id: t.id.clone(),
                text: "hello".into(),
                focus_context: None,
            })
            .await
            .unwrap();

        assert_eq!(user.role, ChatRole::User);
        assert_eq!(user.status, MessageStatus::Complete);
        assert_eq!(user.content_text, "hello");
        assert_eq!(user.content_blocks.len(), 1);
        assert!(matches!(
            &user.content_blocks[0],
            ContentBlock::Text { text } if text == "hello"
        ));
        assert!(user.cited_frame_ids.is_none());

        // by_thread sees it without finalize ever running.
        let listed = messages.by_thread(&t.id).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, user.id);
    }

    #[tokio::test]
    async fn quick_ask_user_message_includes_focus_block_and_citation() {
        let pool = init_pool();
        let threads = SqliteChatThreadRepo::new(pool.clone());
        let messages = SqliteChatMessageRepo::new(pool);

        let t = threads.create(fake_thread(None)).await.unwrap();
        let user = messages
            .insert_user_message(NewUserMessage {
                thread_id: t.id.clone(),
                text: "这是什么".into(),
                focus_context: Some(UserFocusContext {
                    frame_id: "F1".into(),
                    summary: "VSCode".into(),
                    primary_text: Some("fn foo() {}".into()),
                    selection: None,
                }),
            })
            .await
            .unwrap();

        assert_eq!(
            user.cited_frame_ids.as_deref(),
            Some(&["F1".to_string()][..])
        );
        assert_eq!(user.content_blocks.len(), 2);
        assert!(matches!(
            &user.content_blocks[0],
            ContentBlock::FocusContext { frame_id, .. } if frame_id == "F1"
        ));
        assert!(matches!(
            &user.content_blocks[1],
            ContentBlock::Text { text } if text == "这是什么"
        ));
    }

    #[tokio::test]
    async fn streaming_placeholder_is_hidden_then_finalize_surfaces_it() {
        let pool = init_pool();
        let threads = SqliteChatThreadRepo::new(pool.clone());
        let messages = SqliteChatMessageRepo::new(pool);

        let t = threads.create(fake_thread(None)).await.unwrap();
        messages
            .insert_user_message(NewUserMessage {
                thread_id: t.id.clone(),
                text: "go".into(),
                focus_context: None,
            })
            .await
            .unwrap();

        let placeholder = messages.start_assistant_message(&t.id).await.unwrap();
        assert_eq!(placeholder.status, MessageStatus::Streaming);
        // by_thread filters streaming rows out — only user is visible.
        let listed = messages.by_thread(&t.id).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].role, ChatRole::User);

        let final_blocks = vec![
            ContentBlock::Thinking {
                text: "let me think".into(),
                signature: Some("sig_xyz".into()),
            },
            ContentBlock::Text {
                text: "answer".into(),
            },
        ];
        let usage = Usage {
            input_tokens: 100,
            output_tokens: 25,
            ..Default::default()
        };
        let finalized = messages
            .finalize_assistant_message(FinalizeAssistant {
                message_id: placeholder.id.clone(),
                content_blocks: final_blocks.clone(),
                cited_frame_ids: vec![],
                status: MessageStatus::Complete,
                error_message: None,
                finish_reason: Some("EndTurn".into()),
                usage: Some(usage.clone()),
            })
            .await
            .unwrap();

        assert_eq!(finalized.status, MessageStatus::Complete);
        assert_eq!(finalized.content_blocks, final_blocks);
        assert_eq!(finalized.content_text, "let me think\nanswer");
        assert_eq!(finalized.usage.as_ref(), Some(&usage));
        assert_eq!(finalized.finish_reason.as_deref(), Some("EndTurn"));
        assert!(finalized.updated_at.is_some());

        let listed = messages.by_thread(&t.id).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].role, ChatRole::User);
        assert_eq!(listed[1].id, placeholder.id);
    }

    #[tokio::test]
    async fn finalize_with_error_status_keeps_partial_blocks() {
        let pool = init_pool();
        let threads = SqliteChatThreadRepo::new(pool.clone());
        let messages = SqliteChatMessageRepo::new(pool);

        let t = threads.create(fake_thread(None)).await.unwrap();
        messages
            .insert_user_message(NewUserMessage {
                thread_id: t.id.clone(),
                text: "go".into(),
                focus_context: None,
            })
            .await
            .unwrap();
        let placeholder = messages.start_assistant_message(&t.id).await.unwrap();

        let partial = vec![ContentBlock::Text {
            text: "partial answer before crash".into(),
        }];
        let finalized = messages
            .finalize_assistant_message(FinalizeAssistant {
                message_id: placeholder.id.clone(),
                content_blocks: partial.clone(),
                cited_frame_ids: vec![],
                status: MessageStatus::Error,
                error_message: Some("upstream 500".into()),
                finish_reason: Some("Error".into()),
                usage: None,
            })
            .await
            .unwrap();

        assert_eq!(finalized.status, MessageStatus::Error);
        assert_eq!(finalized.error_message.as_deref(), Some("upstream 500"));
        assert_eq!(finalized.content_blocks, partial);
        // Both rows visible — the user's prompt survived, error and all.
        let listed = messages.by_thread(&t.id).await.unwrap();
        assert_eq!(listed.len(), 2);
    }

    #[tokio::test]
    async fn cancel_streaming_orphans_flips_status_and_returns_count() {
        let pool = init_pool();
        let threads = SqliteChatThreadRepo::new(pool.clone());
        let messages = SqliteChatMessageRepo::new(pool);

        let t = threads.create(fake_thread(None)).await.unwrap();
        // Two orphan placeholders (e.g. from a crash).
        messages.start_assistant_message(&t.id).await.unwrap();
        messages.start_assistant_message(&t.id).await.unwrap();
        // Plus a complete message that should NOT be touched.
        messages
            .insert_user_message(NewUserMessage {
                thread_id: t.id.clone(),
                text: "untouched".into(),
                focus_context: None,
            })
            .await
            .unwrap();

        let n = messages.cancel_streaming_orphans().await.unwrap();
        assert_eq!(n, 2);

        let listed = messages.by_thread(&t.id).await.unwrap();
        // Both former-streaming rows now visible as cancelled.
        assert_eq!(listed.len(), 3);
        let cancelled_count = listed
            .iter()
            .filter(|m| m.status == MessageStatus::Cancelled)
            .count();
        assert_eq!(cancelled_count, 2);
    }

    #[tokio::test]
    async fn delete_thread_cascades_messages() {
        let pool = init_pool();
        let threads = SqliteChatThreadRepo::new(pool.clone());
        let messages = SqliteChatMessageRepo::new(pool);

        let t = threads.create(fake_thread(None)).await.unwrap();
        for content in ["a", "b", "c"] {
            messages
                .insert_user_message(NewUserMessage {
                    thread_id: t.id.clone(),
                    text: content.into(),
                    focus_context: None,
                })
                .await
                .unwrap();
        }
        assert_eq!(messages.by_thread(&t.id).await.unwrap().len(), 3);

        threads.delete(&t.id).await.unwrap();
        assert!(threads.by_id(&t.id).await.unwrap().is_none());
        assert_eq!(messages.by_thread(&t.id).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn message_fts_search_excludes_system_thread_and_streaming() {
        let pool = init_pool();
        let threads = SqliteChatThreadRepo::new(pool.clone());
        let messages = SqliteChatMessageRepo::new(pool);

        // One user thread with a complete message we should find.
        let user_t = threads.create(fake_thread(Some("debug"))).await.unwrap();
        messages
            .insert_user_message(NewUserMessage {
                thread_id: user_t.id.clone(),
                text: "用 useReducer 替代 useState 的死循环".into(),
                focus_context: None,
            })
            .await
            .unwrap();

        // System thread with the same text — must be invisible.
        let sys_t = threads
            .create(NewChatThread {
                title: Some("[learner]".into()),
                bound_model_id: "claude-sonnet-4-6".into(),
                bound_api_shape: ApiShape::Anthropic,
                kind: ChatThreadKind::System,
                system_task: Some(SystemTaskKind::SessionMemoryLearning),
            })
            .await
            .unwrap();
        messages
            .insert_user_message(NewUserMessage {
                thread_id: sys_t.id.clone(),
                text: "用 useReducer 替代 useState".into(),
                focus_context: None,
            })
            .await
            .unwrap();

        let q = crate::services::tokenize::tokenize_for_query("useReducer").unwrap();
        let hits = messages.fts_search(&q, 10).await.unwrap();
        assert!(hits.iter().all(|h| h.message.thread_id != sys_t.id));
        assert!(hits.iter().any(|h| h.message.thread_id == user_t.id));
    }

    #[tokio::test]
    async fn list_excludes_system_threads_but_list_system_finds_them() {
        let pool = init_pool();
        let threads = SqliteChatThreadRepo::new(pool.clone());
        let _user = threads.create(fake_thread(Some("user-1"))).await.unwrap();
        let sys = threads
            .create(NewChatThread {
                title: Some("[session_memory_learning] X".into()),
                bound_model_id: "claude-sonnet-4-6".into(),
                bound_api_shape: ApiShape::Anthropic,
                kind: ChatThreadKind::System,
                system_task: Some(SystemTaskKind::SessionMemoryLearning),
            })
            .await
            .unwrap();
        let user_list = threads.list(20).await.unwrap();
        assert_eq!(user_list.len(), 1);
        assert_eq!(user_list[0].kind, ChatThreadKind::User);
        let system_list = threads
            .list_system(Some(SystemTaskKind::SessionMemoryLearning), 20)
            .await
            .unwrap();
        assert_eq!(system_list.len(), 1);
        assert_eq!(system_list[0].id, sys.id);
        assert_eq!(system_list[0].kind, ChatThreadKind::System);
        assert_eq!(
            system_list[0].system_task,
            Some(SystemTaskKind::SessionMemoryLearning)
        );
    }

    #[tokio::test]
    async fn update_summary_then_search_returns_thread() {
        let pool = init_pool();
        let threads = SqliteChatThreadRepo::new(pool.clone());
        let t = threads
            .create(fake_thread(Some("性能优化讨论")))
            .await
            .unwrap();
        threads
            .update_summary(ThreadSummaryUpdate {
                thread_id: t.id.clone(),
                summary: "用户和 Corivo 一起调试 useEffect 死循环，最终定位到漏写依赖项。".into(),
                summary_topics: "useEffect 死循环 依赖项 React hooks".into(),
            })
            .await
            .unwrap();
        let hits = threads.search_summaries("useEffect", 5).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].thread_id, t.id);
        assert!(hits[0].score > 0.0);
    }

    #[tokio::test]
    async fn search_summaries_excludes_system_threads() {
        let pool = init_pool();
        let threads = SqliteChatThreadRepo::new(pool.clone());
        let sys = threads
            .create(NewChatThread {
                title: Some("[learner] X".into()),
                bound_model_id: "claude-sonnet-4-6".into(),
                bound_api_shape: ApiShape::Anthropic,
                kind: ChatThreadKind::System,
                system_task: Some(SystemTaskKind::SessionMemoryLearning),
            })
            .await
            .unwrap();
        // We can write summary directly via UPDATE because the public
        // API gates the trigger by kind='user'; this targets the
        // protection.
        threads
            .update_summary(ThreadSummaryUpdate {
                thread_id: sys.id.clone(),
                summary: "useEffect 死循环".into(),
                summary_topics: "useEffect".into(),
            })
            .await
            .unwrap();
        let hits = threads.search_summaries("useEffect", 5).await.unwrap();
        assert!(hits.iter().all(|h| h.thread_id != sys.id));
    }

    #[tokio::test]
    async fn create_then_list_orders_by_recent_update() {
        let pool = init_pool();
        let threads = SqliteChatThreadRepo::new(pool.clone());
        let messages = SqliteChatMessageRepo::new(pool);

        let a = threads.create(fake_thread(Some("first"))).await.unwrap();
        let b = threads.create(fake_thread(Some("second"))).await.unwrap();
        // Posting a user message into `a` bumps its updated_at.
        messages
            .insert_user_message(NewUserMessage {
                thread_id: a.id.clone(),
                text: "hi".into(),
                focus_context: None,
            })
            .await
            .unwrap();

        let list = threads.list(10).await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, a.id);
        assert_eq!(list[1].id, b.id);
    }
}
