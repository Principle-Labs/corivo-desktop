//! `BackgroundAgentTask` impl for persona distillation
//! (memory-system-spec §4.3 / §11.4).

use async_trait::async_trait;

use crate::db::repos::notes::ListNotesOptions;
use crate::domain::chat::SystemTaskKind;
use crate::domain::note::{NoteScope, NoteSourceType, NoteStatus};
use crate::error::Result;
use crate::services::background_agent_task::{runner::TaskOutcome, BackgroundAgentTask, TaskDeps};
use crate::services::persona::{conflict, renderer};

const SYSTEM_PROMPT: &str = include_str!("../../../prompts/persona_distill.md");

pub const TASK_NAME: &str = "persona_distill";

pub const TOOL_WHITELIST: &[&str] = &[
    "memory_search",
    "thread_search",
    "chat_thread_get",
    "note_list",
    "auto_persona_get_previous",
];

pub struct PersonaDistillTask {
    pub evidence_window_days: u32,
}

impl PersonaDistillTask {
    pub fn new(evidence_window_days: u32) -> Self {
        Self {
            evidence_window_days,
        }
    }
}

#[async_trait]
impl BackgroundAgentTask for PersonaDistillTask {
    fn kind(&self) -> SystemTaskKind {
        SystemTaskKind::PersonaDistill
    }

    fn system_prompt(&self) -> String {
        SYSTEM_PROMPT.to_string()
    }

    fn initial_user_message(&self) -> String {
        format!(
            "生成本期记忆画像。证据窗口：最近 {} 天。当前时区：Asia/Shanghai。\n\
             先调用 `auto_persona_get_previous()` 看上一版结果(决定哪些章节要更新)，\
             再调 `note_list(scope=\"global\", status=\"active\", source=\"user_explicit\")` \
             把第一节 \"明确告诉过我的事\" 原样填好，最后按系统提示规定的 Markdown 形式输出。",
            self.evidence_window_days
        )
    }

    fn tool_whitelist(&self) -> &'static [&'static str] {
        TOOL_WHITELIST
    }

    fn max_turns(&self) -> u32 {
        30
    }

    async fn consume_output(
        &self,
        output: String,
        outcome: &TaskOutcome,
        deps: &TaskDeps,
    ) -> Result<()> {
        if !outcome.success {
            tracing::info!("persona_distill.skip_consume_on_failure");
            return Ok(());
        }
        let trimmed = strip_code_fence(&output);
        if trimmed.is_empty() {
            tracing::warn!("persona_distill.empty_output_skipped");
            return Ok(());
        }
        // Sanity check: a real persona has at least the first heading.
        if !trimmed.contains("## ") {
            tracing::warn!(
                preview = %trimmed.chars().take(160).collect::<String>(),
                "persona_distill.output_missing_headings"
            );
            return Ok(());
        }
        // Pull active global notes and annotate conflicts.
        let notes = deps
            .notes_repo
            .list(ListNotesOptions {
                scope: Some(NoteScope::Global),
                status: Some(NoteStatus::Active),
                source_type: Some(NoteSourceType::UserExplicit),
                ..Default::default()
            })
            .await
            .unwrap_or_default();
        let annotated = conflict::annotate_conflicts(trimmed, &notes);
        renderer::atomic_write(&deps.app_data_dir, &annotated)?;
        tracing::info!(
            chars = annotated.chars().count(),
            "persona_distill.persisted"
        );
        Ok(())
    }
}

fn strip_code_fence(raw: &str) -> &str {
    let stripped = raw.trim();
    if let Some(rest) = stripped.strip_prefix("```markdown") {
        return rest.trim().trim_end_matches("```").trim();
    }
    if let Some(rest) = stripped.strip_prefix("```md") {
        return rest.trim().trim_end_matches("```").trim();
    }
    if let Some(rest) = stripped.strip_prefix("```") {
        return rest.trim().trim_end_matches("```").trim();
    }
    stripped
}
