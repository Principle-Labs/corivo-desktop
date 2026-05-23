//! `BackgroundAgentTask` impl for session memory learning
//! (memory-system-spec §3.4.2 / §12.3).
//!
//! The task targets one chat thread that has been idle long enough.
//! After the sidecar returns, we expect a single JSON object in the
//! final assistant text. We parse it; on success the `notes` go into
//! the `notes` table and the `thread_summary` lands on the chat_threads
//! row's summary columns. The checkpoint advances to the latest message
//! id in the source thread either way (failed runs don't get re-tried
//! against the same window — the next idle period will pick up after
//! the new last_processed_id).

use async_trait::async_trait;
use serde::Deserialize;

use crate::db::repos::chat::ThreadSummaryUpdate;
use crate::db::repos::notes::NewNote;
use crate::domain::chat::SystemTaskKind;
use crate::domain::note::{NoteScope, NoteSourceType};
use crate::error::Result;
use crate::services::background_agent_task::{runner::TaskOutcome, BackgroundAgentTask, TaskDeps};
use crate::services::tokenize;

const SYSTEM_PROMPT: &str = include_str!("../../../prompts/session_memory_learning.md");

pub const TASK_NAME: &str = "session_memory_learning";

pub const TOOL_WHITELIST: &[&str] = &["chat_thread_get", "note_list"];

pub struct SessionLearnerTask {
    pub thread_id: String,
    pub after_message_id: Option<String>,
}

impl SessionLearnerTask {
    pub fn new(thread_id: String, after_message_id: Option<String>) -> Self {
        Self {
            thread_id,
            after_message_id,
        }
    }
}

#[async_trait]
impl BackgroundAgentTask for SessionLearnerTask {
    fn kind(&self) -> SystemTaskKind {
        SystemTaskKind::SessionMemoryLearning
    }

    fn system_prompt(&self) -> String {
        SYSTEM_PROMPT.to_string()
    }

    fn initial_user_message(&self) -> String {
        match &self.after_message_id {
            Some(checkpoint) => format!(
                "学习 thread `{}` 自 message `{}` 之后(不含)的对话。\n\
                 先调 `chat_thread_get(thread_id=\"{}\", after_message_id=\"{}\")` \
                 拿全部新消息，再调 `note_list(scope=\"global\", status=\"active\")` \
                 检查已有偏好，最后按系统提示规定的 JSON 形式返回结果。",
                self.thread_id, checkpoint, self.thread_id, checkpoint
            ),
            None => format!(
                "学习 thread `{}` 的全部对话。\n\
                 先调 `chat_thread_get(thread_id=\"{}\")` 拿全部消息，\
                 再调 `note_list(scope=\"global\", status=\"active\")` \
                 检查已有偏好，最后按系统提示规定的 JSON 形式返回结果。",
                self.thread_id, self.thread_id
            ),
        }
    }

    fn tool_whitelist(&self) -> &'static [&'static str] {
        TOOL_WHITELIST
    }

    fn max_turns(&self) -> u32 {
        10
    }

    async fn consume_output(
        &self,
        output: String,
        outcome: &TaskOutcome,
        deps: &TaskDeps,
    ) -> Result<()> {
        if !outcome.success {
            tracing::info!(
                thread_id = %self.thread_id,
                "session_learner.skip_consume_on_failure"
            );
            return Ok(());
        }
        let parsed = match parse_output(&output) {
            Some(p) => p,
            None => {
                tracing::warn!(
                    thread_id = %self.thread_id,
                    output_preview = %output.chars().take(200).collect::<String>(),
                    "session_learner.unparseable_json"
                );
                return Ok(());
            }
        };

        // Persist any notes the learner produced.
        for note in &parsed.notes {
            let scope = NoteScope::parse(&note.scope).unwrap_or(NoteScope::Global);
            let source_type =
                NoteSourceType::parse(&note.source).unwrap_or(NoteSourceType::AgentInferred);
            let new_note = NewNote {
                content: note.content.trim().to_string(),
                scope,
                scope_ref: None,
                source_type,
                source_message_id: note.source_message_id.clone(),
                source_thread_id: Some(self.thread_id.clone()),
                confidence: note.confidence,
                status: None,
                expires_at: None,
            };
            if let Err(error) = deps.notes_repo.create(new_note).await {
                tracing::warn!(?error, "session_learner.note_persist_failed");
            }
        }

        // Persist the thread summary if the learner produced one.
        if let Some(summary) = &parsed.thread_summary {
            if !summary.content.trim().is_empty() {
                let topics_str = summary.topics.join(" ");
                // Pre-tokenize topics for FTS5 (CJK-friendly).
                let topics_tokenized = tokenize::tokenize_for_index(&topics_str);
                let summary_tokens = tokenize::tokenize_for_index(&summary.content);
                let merged_topics = if summary_tokens.is_empty() {
                    topics_tokenized
                } else if topics_tokenized.is_empty() {
                    summary_tokens
                } else {
                    format!("{summary_tokens} {topics_tokenized}")
                };
                if let Err(error) = deps
                    .chat_threads
                    .update_summary(ThreadSummaryUpdate {
                        thread_id: self.thread_id.clone(),
                        summary: summary.content.trim().to_string(),
                        summary_topics: merged_topics,
                    })
                    .await
                {
                    tracing::warn!(?error, "session_learner.summary_persist_failed");
                }
            }
        }

        // Advance the checkpoint to the current latest message — even
        // if the learner returned no notes (a "learned and there's
        // nothing interesting here" run still moves the marker).
        if let Some(latest) = deps
            .chat_threads
            .latest_message_id(&self.thread_id)
            .await
            .ok()
            .flatten()
        {
            if let Err(error) = upsert_checkpoint(deps, &self.thread_id, &latest).await {
                tracing::warn!(
                    ?error,
                    thread_id = %self.thread_id,
                    "session_learner.checkpoint_persist_failed"
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct SessionLearnerOutput {
    #[serde(default)]
    notes: Vec<RawNote>,
    #[serde(default)]
    thread_summary: Option<RawSummary>,
}

#[derive(Debug, Deserialize)]
struct RawNote {
    content: String,
    #[serde(default = "default_scope")]
    scope: String,
    #[serde(default = "default_source")]
    source: String,
    #[serde(default)]
    confidence: Option<f32>,
    #[serde(default)]
    source_message_id: Option<String>,
}

fn default_scope() -> String {
    "global".to_string()
}

fn default_source() -> String {
    "agent_inferred".to_string()
}

#[derive(Debug, Deserialize)]
struct RawSummary {
    content: String,
    #[serde(default)]
    topics: Vec<String>,
}

fn parse_output(text: &str) -> Option<SessionLearnerOutput> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    // The prompt asks for raw JSON; in practice models sometimes still
    // wrap it in ```json fences. Strip a single leading/trailing fence
    // pair before deserializing.
    let candidate = strip_code_fence(trimmed);
    serde_json::from_str::<SessionLearnerOutput>(candidate).ok()
}

fn strip_code_fence(raw: &str) -> &str {
    let stripped = raw.trim();
    if let Some(rest) = stripped.strip_prefix("```json") {
        return rest.trim().trim_end_matches("```").trim();
    }
    if let Some(rest) = stripped.strip_prefix("```") {
        return rest.trim().trim_end_matches("```").trim();
    }
    stripped
}

async fn upsert_checkpoint(
    deps: &TaskDeps,
    thread_id: &str,
    last_processed_id: &str,
) -> Result<()> {
    use crate::db::pool::run_blocking;
    use crate::db::time::DbInstant;
    use rusqlite::params;

    let task = TASK_NAME.to_string();
    let target = thread_id.to_string();
    let last_id = last_processed_id.to_string();
    let now = DbInstant::now();
    run_blocking(deps.db_pool.clone(), move |conn| {
        conn.execute(
            "INSERT INTO background_agent_task_checkpoints
               (task, target_id, last_processed_id, last_run_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(task, target_id) DO UPDATE SET
               last_processed_id = excluded.last_processed_id,
               last_run_at = excluded.last_run_at",
            params![task, target, last_id, now],
        )
        .map_err(|e| crate::error::CorivoError::Internal(format!("upsert checkpoint 失败: {e}")))?;
        Ok(())
    })
    .await
}

/// Look up the last `message_id` the session learner has already
/// processed for `thread_id`. `None` means the thread has never been
/// learned from (the next run pulls every message).
pub async fn get_checkpoint(deps: &TaskDeps, thread_id: &str) -> Option<String> {
    use crate::db::pool::run_blocking;
    use rusqlite::{params, OptionalExtension};

    let task = TASK_NAME.to_string();
    let target = thread_id.to_string();
    run_blocking(deps.db_pool.clone(), move |conn| {
        let row: Option<Option<String>> = conn
            .query_row(
                "SELECT last_processed_id FROM background_agent_task_checkpoints
                 WHERE task = ?1 AND target_id = ?2",
                params![task, target],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|e| {
                crate::error::CorivoError::Internal(format!("get checkpoint 失败: {e}"))
            })?;
        Ok(row.flatten())
    })
    .await
    .ok()
    .flatten()
}
