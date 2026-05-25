//! `ScheduledWorkflowTask` — adapter from a stored
//! [`WorkflowDefinition`] + run metadata into a
//! [`BackgroundAgentTask`].
//!
//! The Ticker constructs one of these per due schedule and enqueues
//! it onto the shared `BackgroundAgentScheduler`. We don't touch the
//! schedule row here — the Ticker already advanced `next_run_at`
//! before enqueueing. What we *do* own is the audit trail: writing
//! the `workflow_runs` row and bumping `last_run_at` / `last_status`
//! on the schedule when the sidecar returns.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::db::time::now_utc;
use crate::domain::chat::SystemTaskKind;
use crate::domain::workflow::{
    WorkflowDefinition, WorkflowNotifyPolicy, WorkflowRun, WorkflowRunStatus,
};
use crate::error::Result;
use crate::services::background_agent_task::{
    runner::TaskOutcome, BackgroundAgentTask, TaskDeps,
};
use crate::services::scheduled_workflows::store::WorkflowStore;

pub struct ScheduledWorkflowTask {
    /// Workflow slug — matches `workflow_schedules.slug` and the
    /// directory name under `$APPDATA/corivo/workflows/`.
    slug: String,
    /// Snapshot of the WORKFLOW.md content. Taken at dispatch time so
    /// editing the file mid-run doesn't churn the in-flight prompt.
    definition: WorkflowDefinition,
    /// Pre-generated ULID for this run. The Ticker hands it down so
    /// the workflow_runs row can be planned before the sidecar
    /// actually starts (useful for forthcoming "in-flight" UI states).
    run_id: String,
    /// When the Ticker pulled this schedule off the queue. Stored as
    /// `started_at` on the run row. For v1's single-flight scheduler
    /// this is within a couple of seconds of the actual sidecar
    /// kickoff so we don't bother capturing a separate "begin work"
    /// timestamp inside the runner.
    started_at: DateTime<Utc>,
    /// Used in `consume_output` to record the run + bump the
    /// schedule's `last_run_at` / `last_status`.
    store: Arc<WorkflowStore>,
}

impl ScheduledWorkflowTask {
    pub fn new(
        slug: String,
        definition: WorkflowDefinition,
        run_id: String,
        started_at: DateTime<Utc>,
        store: Arc<WorkflowStore>,
    ) -> Self {
        Self {
            slug,
            definition,
            run_id,
            started_at,
            store,
        }
    }
}

#[async_trait]
impl BackgroundAgentTask for ScheduledWorkflowTask {
    fn kind(&self) -> SystemTaskKind {
        SystemTaskKind::ScheduledWorkflow
    }

    fn system_prompt(&self) -> String {
        render_template(&self.definition.system_prompt, self.started_at)
    }

    fn initial_user_message(&self) -> String {
        // The system prompt does the heavy lifting; the user-side
        // message just kicks the agent off. Workflow authors can hint
        // at "execute now" semantics in the prompt itself.
        format!(
            "请按上方系统提示执行 workflow `{}`。运行时间：{} UTC。",
            self.slug,
            self.started_at.format("%Y-%m-%d %H:%M:%S")
        )
    }

    fn tool_whitelist(&self) -> Vec<String> {
        self.definition.tool_whitelist.clone()
    }

    fn max_turns(&self) -> u32 {
        self.definition.max_turns
    }

    fn cancellation_key(&self) -> Option<String> {
        // Slug = stable per-workflow identifier. The cancel IPC and
        // the run_now/cron dedup tracker both key on slug, so a
        // single Cancel button can hit either an in-flight task or
        // a queued one without distinguishing.
        Some(self.slug.clone())
    }

    fn before_run(&self, deps: &TaskDeps, _thread_id: &str) {
        // The sidecar turn itself can take 10–120 seconds for an
        // LLM-heavy workflow; the frontend uses this event to flip
        // the row into "正在运行..." and pin the toast so the user
        // doesn't think nothing happened.
        crate::services::scheduled_workflows::notify::dispatch_started(
            &deps.app,
            &self.definition.name,
            &self.slug,
            &self.run_id,
        );
    }

    fn on_dispatch_aborted(&self, reason: &str) {
        // Scheduler gave up before runner::run could complete (deps
        // never became ready, runner crashed, etc.). Without this
        // hook, the frontend would show an in-flight spinner forever
        // because no completed event ever fires.
        //
        // Best-effort: release the dedup claim, write a failure run
        // row so /workflows/<slug>/history shows what happened, and
        // emit workflow:completed so the spinner clears + a toast
        // surfaces the reason.
        tracing::warn!(
            slug = %self.slug,
            run_id = %self.run_id,
            reason,
            "scheduled_workflow.dispatch_aborted"
        );
        self.store.release_slug(&self.slug);

        let store = self.store.clone();
        let slug = self.slug.clone();
        let name = self.definition.name.clone();
        let run_id = self.run_id.clone();
        let started_at = self.started_at;
        let reason_owned = reason.to_string();
        tauri::async_runtime::spawn(async move {
            use crate::domain::workflow::{WorkflowRun, WorkflowRunStatus};
            // Specialize the user-facing summary: a user-initiated
            // cancellation isn't really a "failure" in the way a
            // crash is, even though we still record status=failure
            // for lack of a `cancelled` enum variant. The summary
            // text is what shows up in the toast + sidebar, so it
            // matters that it reads as "已取消" not "未能启动".
            let summary = if reason_owned == "cancelled_by_user" {
                "已取消".to_string()
            } else {
                format!("未能启动 — {reason_owned}")
            };
            let run = WorkflowRun {
                id: run_id.clone(),
                slug: slug.clone(),
                thread_id: None,
                status: WorkflowRunStatus::Failure,
                started_at,
                finished_at: crate::db::time::now_utc(),
                error_message: Some(format!(
                    "scheduler aborted dispatch: {reason_owned}"
                )),
                summary: Some(summary.clone()),
                content_hash: None,
                acknowledged_at: None,
            };
            if let Err(error) = store.record_run(run).await {
                tracing::warn!(
                    slug = %slug,
                    ?error,
                    "scheduled_workflow.dispatch_aborted.record_run_failed"
                );
            }
            // Emit workflow:completed so the frontend listener can
            // dismiss the in-flight spinner + transition the toast.
            // Without this the user stares at "正在运行..." forever.
            if let Some(app) = store.app_handle() {
                crate::services::scheduled_workflows::notify::dispatch(
                    app, &name, &slug, &run_id, "", &summary, false,
                );
            }
        });
    }

    async fn consume_output(
        &self,
        output: String,
        outcome: &TaskOutcome,
        deps: &TaskDeps,
    ) -> Result<()> {
        let status = if outcome.success {
            WorkflowRunStatus::Success
        } else {
            WorkflowRunStatus::Failure
        };
        let summary =
            summarize_for_notification(&output, outcome.success, outcome.error_detail.as_deref());
        let content_hash = if outcome.success {
            Some(hash_content(&output))
        } else {
            None
        };

        // notify_policy = on_change skips push when the hash matches
        // the previous successful run. The DB row is still written
        // either way (so it shows in the sidebar with the unread dot).
        let suppress_push = if outcome.success
            && self.definition.notify_policy == WorkflowNotifyPolicy::OnChange
        {
            match self.store.latest_run_for_slug(&self.slug).await {
                Ok(Some(prev)) => prev.content_hash.as_deref() == content_hash.as_deref(),
                _ => false,
            }
        } else {
            false
        };

        let run = WorkflowRun {
            id: self.run_id.clone(),
            slug: self.slug.clone(),
            thread_id: Some(outcome.thread_id.clone()),
            status,
            started_at: self.started_at,
            finished_at: now_utc(),
            // Carry the runner's specific failure reason (timeout vs.
            // sidecar error vs. join error) into the DB so the history
            // dialog can show what actually went wrong. Fall back to a
            // generic message only when the runner produced no detail
            // (shouldn't happen on the !success path, but stays
            // defensive).
            error_message: if outcome.success {
                None
            } else {
                Some(
                    outcome
                        .error_detail
                        .clone()
                        .unwrap_or_else(|| "运行未正常结束".to_string()),
                )
            },
            summary: Some(summary.clone()),
            content_hash: content_hash.clone(),
            acknowledged_at: None,
        };
        // Record the run row, but DO NOT abort on persistence failure.
        // The previous early-return left the frontend hanging on "正在
        // 运行..." indefinitely: it expected a `workflow:completed`
        // event (emitted below), but Err short-circuited past the
        // emit. The slug release also has to fire regardless so the
        // next dispatch attempt isn't locked out.
        //
        // Real incident 2026-05-23 10:53:53: watchdog fired, runner
        // bailed, consume_output called → record_run hit a FOREIGN
        // KEY constraint (schedule-less run-now path; see v1433
        // migration) → early Err → notify::dispatch skipped → UI
        // stuck. Backend was clean, frontend wasn't.
        let persist_error = match self.store.record_run(run.clone()).await {
            Ok(()) => None,
            Err(error) => {
                tracing::warn!(
                    slug = %self.slug,
                    run_id = %self.run_id,
                    ?error,
                    "scheduled_workflow.record_run_failed"
                );
                Some(error)
            }
        };
        // Release the dedup claim regardless of persistence outcome —
        // the next cron tick / run-now click should be able to proceed.
        self.store.release_slug(&self.slug);

        // Push the macOS banner + emit the Tauri event. Failures are
        // always pushed (the user wants to know things broke);
        // successes obey notify_policy. Both code paths leave the
        // workflow_runs row + sidebar dot intact, so a suppressed run
        // is still discoverable, just not noisy.
        let should_push = match (outcome.success, self.definition.notify_policy) {
            (false, _) => true,
            (true, WorkflowNotifyPolicy::Silent) => false,
            (true, WorkflowNotifyPolicy::OnChange) => !suppress_push,
            (true, WorkflowNotifyPolicy::Always) => true,
        };
        if should_push {
            crate::services::scheduled_workflows::notify::dispatch(
                &deps.app,
                &self.definition.name,
                &self.slug,
                &self.run_id,
                outcome.thread_id.as_str(),
                &summary,
                outcome.success,
            );
        } else {
            tracing::debug!(
                slug = %self.slug,
                run_id = %self.run_id,
                "scheduled_workflow.notify_suppressed"
            );
        }

        tracing::info!(
            slug = %self.slug,
            run_id = %self.run_id,
            success = outcome.success,
            pushed = should_push,
            persisted = persist_error.is_none(),
            "scheduled_workflow.recorded"
        );
        // Surface the persistence failure to the runner so the per-task
        // log + `background_agent_task.consume_output_failed` tracing
        // span still capture it. The frontend has already been notified
        // (notify::dispatch above ran before this return), so the UI
        // doesn't depend on Ok here.
        match persist_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

/// Truncate the assistant final text to ~140 chars for the
/// notification body. Failures fall back to the runner-supplied
/// `error_detail` (timeout reason, sidecar error, etc.) since the
/// `output` buffer is usually empty when the run errored before
/// producing a final answer.
///
/// Old version returned a fixed "运行失败 — 点击查看错误详情" string
/// that promised an interaction (a click) the history dialog didn't
/// actually deliver — a Nielsen visibility-of-system-status fail.
fn summarize_for_notification(output: &str, success: bool, error_detail: Option<&str>) -> String {
    if !success {
        return match error_detail {
            Some(detail) if !detail.trim().is_empty() => {
                truncate_summary(detail.trim())
            }
            _ => "运行失败".to_string(),
        };
    }
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return "本次运行未产出内容".to_string();
    }
    truncate_summary(trimmed)
}

/// Char-based truncation to ~140 chars (the body may be CJK; byte
/// slicing would panic on a multi-byte boundary).
fn truncate_summary(text: &str) -> String {
    let mut iter = text.chars();
    let head: String = iter.by_ref().take(140).collect();
    if iter.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

/// Hex-encoded SHA-256 of the raw assistant output. Used by
/// `notify_policy = 'on_change'` to detect a re-emitted-identical run.
fn hash_content(output: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(output.as_bytes());
    hex::encode(hasher.finalize())
}

/// Render `{{slot}}` placeholders inside the workflow's system-prompt
/// template. v1 supports two slots:
///
/// * `{{date}}` — today's UTC date (`YYYY-MM-DD`).
/// * `{{date_yesterday}}` — UTC date minus one day. Common for
///   "daily review" style workflows that pull the previous day's
///   frames.
///
/// Additional slots land with the editor UI; unknown slots are left
/// untouched so a forward-compatible WORKFLOW.md keeps rendering
/// when an older app version reads it.
fn render_template(template: &str, run_at: DateTime<Utc>) -> String {
    let today = run_at.format("%Y-%m-%d").to_string();
    let yesterday = (run_at - chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();
    template
        .replace("{{date}}", &today)
        .replace("{{date_yesterday}}", &yesterday)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn render_template_substitutes_known_slots() {
        let when = Utc.with_ymd_and_hms(2026, 5, 21, 1, 0, 0).unwrap();
        let out = render_template(
            "纪要日期：{{date}}，回顾窗口：{{date_yesterday}}。",
            when,
        );
        assert_eq!(out, "纪要日期：2026-05-21，回顾窗口：2026-05-20。");
    }

    #[test]
    fn render_template_leaves_unknown_slots_alone() {
        let when = Utc.with_ymd_and_hms(2026, 5, 21, 0, 0, 0).unwrap();
        let out = render_template("待补 {{custom_slot}}", when);
        assert_eq!(out, "待补 {{custom_slot}}");
    }
}
