//! Assemble the corivo-agent sidecar's `system_prompt_extra` (spec §5.1).
//!
//! v1400 (memory-system-spec §6 / §8 / §9): the old `Soul.md` user-edit
//! surface is gone — voice / personality is now expressed through the
//! `<persistent_memory>` block sourced from the `notes` table
//! (`scope='global'`, `source_type='user_explicit'`, `status='active'`).
//!
//! Final assembly order:
//!
//! 1. `<persistent_memory>` — hard user-declared facts. Always-on, no
//!    recall query, hard cap at `PERSISTENT_BUDGET_TOKENS` so a runaway
//!    notes list can't blow the prompt out.
//! 2. `Agent.md` — agent role + working principles. User-editable.
//! 3. `Tools.md` — local tooling memo. User-editable.
//!
//! All three are separated by `---` so a model scanning the appended
//! prompt for distinct sections can spot the boundaries.

use std::path::{Path, PathBuf};

use crate::db::repos::notes::{ListNotesOptions, NotesRepo};
use crate::domain::note::{Note, NoteScope, NoteSourceType, NoteStatus};
use crate::services::memory::{render_relevant_block, MemoryService, RecallRequest};
use crate::services::persona::renderer::read as read_auto_persona;

const AGENT_FILE: &str = "Agent.md";
const TOOLS_FILE: &str = "Tools.md";

const DEFAULT_AGENT_MD: &str = include_str!("../../../prompts/default_agent.md");
const DEFAULT_TOOLS_MD: &str = include_str!("../../../prompts/default_tools.md");

/// Conservative cap on `<persistent_memory>` size. Spec §6.1 calls for
/// 1000 tokens; we approximate one token ≈ 3 chars (CJK-friendly) so
/// 3000 chars is the soft target before truncation kicks in.
const PERSISTENT_BUDGET_CHARS: usize = 3000;

/// Cap on the persona block injection — spec §7.6 calls for 800
/// tokens. 2400 chars ≈ 800 tokens for CJK-heavy content.
const PERSONA_BUDGET_CHARS: usize = 2400;

/// Inputs `load` needs above and beyond the always-present
/// `app_data_dir`. Kept as a struct so future blocks (persona, project
/// context) drop in without a 7-arg call site.
pub struct LoadInputs<'a> {
    pub notes: Option<&'a dyn NotesRepo>,
    /// memory-system-spec §6.2 — when supplied, the loader runs a
    /// `recall()` and stitches a `<relevant_memory>` block into the
    /// final prompt.
    pub memory: Option<&'a MemoryService>,
    pub recall: Option<RecallRequest>,
}

impl<'a> Default for LoadInputs<'a> {
    fn default() -> Self {
        Self {
            notes: None,
            memory: None,
            recall: None,
        }
    }
}

pub async fn load(app_data_dir: &Path, inputs: LoadInputs<'_>) -> Option<String> {
    let mut sections: Vec<String> = Vec::new();

    if let Some(repo) = inputs.notes {
        match repo
            .list(ListNotesOptions {
                scope: Some(NoteScope::Global),
                status: Some(NoteStatus::Active),
                source_type: Some(NoteSourceType::UserExplicit),
                ..Default::default()
            })
            .await
        {
            Ok(rows) if !rows.is_empty() => {
                sections.push(render_persistent_block(&rows));
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(?error, "exec_agent.local_context.notes_load_failed");
            }
        }
    }

    // Soft persona block — read whatever the most recent
    // persona_distill task wrote. Capped at PERSONA_BUDGET_CHARS;
    // we never refuse the chat over a missing or corrupt file, so
    // any read failure is degraded to "skip the block".
    if let Some(body) = read_auto_persona(app_data_dir) {
        let trimmed: String = body.chars().take(PERSONA_BUDGET_CHARS).collect();
        if !trimmed.trim().is_empty() {
            sections.push(format!(
                "<persona_memory>\n<!-- 这是 Corivo 推断出的画像，软约束；如与 persistent_memory 冲突，以 persistent_memory 为准。 -->\n{trimmed}\n</persona_memory>"
            ));
        }
    }

    if let (Some(service), Some(req)) = (inputs.memory, inputs.recall.as_ref()) {
        match service.recall(req).await {
            Ok(result) => {
                if let Some(block) = render_relevant_block(&result.items) {
                    sections.push(block);
                }
                tracing::debug!(
                    target: "exec_agent",
                    note_hits = result.diagnostics.note_hits,
                    frame_hits = result.diagnostics.frame_hits,
                    message_hits = result.diagnostics.message_hits,
                    elapsed_ms = result.diagnostics.elapsed_ms,
                    "exec_agent.local_context.recall_diagnostics"
                );
            }
            Err(error) => {
                tracing::warn!(?error, "exec_agent.local_context.recall_failed");
            }
        }
    }

    let agent_path = app_data_dir.join(AGENT_FILE);
    let tools_path = app_data_dir.join(TOOLS_FILE);
    let agent = ensure_or_read(&agent_path, DEFAULT_AGENT_MD);
    let tools = ensure_or_read(&tools_path, DEFAULT_TOOLS_MD);

    for piece in [agent, tools].into_iter().flatten() {
        let trimmed = piece.trim().to_string();
        if !trimmed.is_empty() {
            sections.push(trimmed);
        }
    }

    if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n\n---\n\n"))
    }
}

/// Render `<persistent_memory>...</persistent_memory>` from a sorted-by-
/// `created_at` list of notes. Oldest notes are kept first; truncation
/// drops from the OLDEST end so the user's most recent declarations
/// survive when the budget gets tight (recent declarations are usually
/// the corrections / refinements the user just made).
fn render_persistent_block(notes: &[Note]) -> String {
    // Lines first so we can size-fit.
    let lines: Vec<String> = notes
        .iter()
        .map(|n| format!("- {}", n.content.trim()))
        .collect();
    // Newest-first: oldest gets dropped if we overflow.
    let mut budget = PERSISTENT_BUDGET_CHARS;
    let mut kept_rev: Vec<&str> = Vec::new();
    for line in lines.iter().rev() {
        let cost = line.chars().count() + 1; // +1 for the joining newline
        if cost > budget && !kept_rev.is_empty() {
            tracing::warn!(
                dropped = notes.len() - kept_rev.len(),
                "exec_agent.local_context.persistent_truncated"
            );
            break;
        }
        budget = budget.saturating_sub(cost);
        kept_rev.push(line.as_str());
    }
    let body = kept_rev.into_iter().rev().collect::<Vec<_>>().join("\n");
    format!(
        "<persistent_memory>\n<!-- 用户明确告诉过 Corivo 要长期记住的偏好 / 事实。任何时候都生效，优先级高于推断画像。 -->\n{body}\n</persistent_memory>"
    )
}

fn ensure_or_read(path: &PathBuf, default_content: &str) -> Option<String> {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "exec_agent.local_context.create_parent_failed"
                );
                return None;
            }
        }
        if let Err(e) = std::fs::write(path, default_content) {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "exec_agent.local_context.seed_default_failed"
            );
            return None;
        }
        tracing::info!(
            path = %path.display(),
            "exec_agent.local_context.seeded_default"
        );
    }
    match std::fs::read_to_string(path) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "exec_agent.local_context.read_failed"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::test_in_memory_pool;
    use crate::db::repos::notes::{NewNote, SqliteNotesRepo};
    use tempfile::TempDir;

    const SCHEMA_SQL: &str = include_str!("../../db/schema.sql");

    fn init_repo() -> SqliteNotesRepo {
        let pool = test_in_memory_pool().unwrap();
        pool.get().unwrap().execute_batch(SCHEMA_SQL).unwrap();
        SqliteNotesRepo::new(pool)
    }

    #[tokio::test]
    async fn no_notes_still_seeds_agent_and_tools() {
        let dir = TempDir::new().unwrap();
        let repo = init_repo();
        let out = load(
            dir.path(),
            LoadInputs {
                notes: Some(&repo),
                memory: None,
                recall: None,
            },
        )
        .await
        .expect("non-empty");
        assert!(dir.path().join(AGENT_FILE).exists());
        assert!(dir.path().join(TOOLS_FILE).exists());
        assert!(out.contains("Corivo Agent"));
        assert!(out.contains("Corivo Tools"));
        // Block opens with a tag immediately followed by the doc-comment
        // marker. Doc comments inside Agent.md mention the tag too, so
        // we look for the rendered block's specific structure.
        assert!(!out.contains("<persistent_memory>\n<!--"));
    }

    #[tokio::test]
    async fn global_active_user_notes_emit_persistent_block() {
        let dir = TempDir::new().unwrap();
        let repo = init_repo();
        repo.create(NewNote {
            content: "用中文回我".into(),
            scope: NoteScope::Global,
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
        repo.create(NewNote {
            content: "别问要不要确认".into(),
            scope: NoteScope::Global,
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
        let out = load(
            dir.path(),
            LoadInputs {
                notes: Some(&repo),
                memory: None,
                recall: None,
            },
        )
        .await
        .expect("non-empty");
        assert!(out.contains("<persistent_memory>\n<!--"));
        assert!(out.contains("- 用中文回我"));
        assert!(out.contains("- 别问要不要确认"));
        // Block sits before Agent.md.
        let persistent_at = out.find("<persistent_memory>\n<!--").unwrap();
        let agent_at = out.find("Corivo Agent").unwrap();
        assert!(persistent_at < agent_at);
    }

    #[tokio::test]
    async fn suggested_notes_do_not_reach_persistent_block() {
        let dir = TempDir::new().unwrap();
        let repo = init_repo();
        repo.create(NewNote {
            content: "可能喜欢 jj".into(),
            scope: NoteScope::Global,
            scope_ref: None,
            source_type: NoteSourceType::AgentInferred,
            source_message_id: None,
            source_thread_id: None,
            confidence: None,
            status: None,
            expires_at: None,
        })
        .await
        .unwrap();
        let out = load(
            dir.path(),
            LoadInputs {
                notes: Some(&repo),
                memory: None,
                recall: None,
            },
        )
        .await
        .expect("non-empty");
        // Block opens with a tag immediately followed by the doc-comment
        // marker. Doc comments inside Agent.md mention the tag too, so
        // we look for the rendered block's specific structure.
        assert!(!out.contains("<persistent_memory>\n<!--"));
        assert!(!out.contains("jj"));
    }

    #[tokio::test]
    async fn session_scope_excluded_from_persistent_block() {
        let dir = TempDir::new().unwrap();
        let repo = init_repo();
        repo.create(NewNote {
            content: "本 thread 只讨论性能".into(),
            scope: NoteScope::Session,
            scope_ref: Some("T1".into()),
            source_type: NoteSourceType::UserExplicit,
            source_message_id: None,
            source_thread_id: Some("T1".into()),
            confidence: None,
            status: None,
            expires_at: None,
        })
        .await
        .unwrap();
        let out = load(
            dir.path(),
            LoadInputs {
                notes: Some(&repo),
                memory: None,
                recall: None,
            },
        )
        .await
        .expect("non-empty");
        // Block opens with a tag immediately followed by the doc-comment
        // marker. Doc comments inside Agent.md mention the tag too, so
        // we look for the rendered block's specific structure.
        assert!(!out.contains("<persistent_memory>\n<!--"));
    }
}
