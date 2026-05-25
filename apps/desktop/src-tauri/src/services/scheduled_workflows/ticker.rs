//! Periodic ticker that dispatches due scheduled workflows.
//!
//! Wakes every [`TICK_INTERVAL`] (60s by default), asks the store for
//! schedules where `enabled = 1 AND next_run_at <= now()`, loads the
//! matching `WORKFLOW.md` from disk, and enqueues a
//! [`ScheduledWorkflowTask`] onto the shared
//! [`BackgroundAgentScheduler`]. Single-flight semantics fall out of
//! the scheduler — multiple due schedules dispatch sequentially in
//! `next_run_at` order.
//!
//! Two correctness invariants:
//!
//! 1. `store.list_and_advance_due` runs in a single SQLite transaction
//!    that *both* selects and advances `next_run_at`. A long-running
//!    workflow can therefore never re-fire from a second ticker pass.
//!
//! 2. A schedule whose `WORKFLOW.md` is missing (user deleted the
//!    file out-of-band) gets auto-disabled here. We log a warning,
//!    flip `enabled = 0`, and never look at it again until the user
//!    re-creates / re-enables.

use std::sync::Arc;
use std::time::Duration;

use tokio::time::sleep;
use ulid::Ulid;

use crate::db::time::now_utc;
use crate::services::background_agent_task::{
    BackgroundAgentScheduler, ScheduleEnqueue,
};
use crate::services::scheduled_workflows::store::WorkflowStore;
use crate::services::scheduled_workflows::task::ScheduledWorkflowTask;

const TICK_INTERVAL: Duration = Duration::from_secs(60);
const STARTUP_WARMUP: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct ScheduledWorkflowTicker {
    store: Arc<WorkflowStore>,
    scheduler: BackgroundAgentScheduler,
}

impl ScheduledWorkflowTicker {
    pub fn new(
        store: Arc<WorkflowStore>,
        scheduler: BackgroundAgentScheduler,
    ) -> Self {
        Self { store, scheduler }
    }

    /// Spawn the tick loop on the Tauri-managed runtime. Returns
    /// immediately. The loop runs forever; there's no shutdown signal
    /// because the app exit takes the runtime down.
    pub fn start(self) -> tauri::async_runtime::JoinHandle<()> {
        tauri::async_runtime::spawn(async move {
            sleep(STARTUP_WARMUP).await;
            tracing::info!("scheduled_workflow.ticker_started");
            loop {
                if let Err(error) = self.tick_once().await {
                    tracing::warn!(?error, "scheduled_workflow.tick_failed");
                }
                sleep(TICK_INTERVAL).await;
            }
        })
    }

    /// One pass. Exposed for tests + the manual "run now" path.
    pub async fn tick_once(&self) -> crate::error::Result<usize> {
        let now = now_utc();
        let due = self.store.list_and_advance_due(now).await?;
        if due.is_empty() {
            return Ok(0);
        }
        let mut dispatched = 0;
        for entry in due {
            // Skip the cron tick if the slug already has a run in
            // flight (previous run hung, or two ticks raced). The
            // schedule's `next_run_at` was already advanced atomically
            // in list_and_advance_due, so we don't double-fire the
            // *schedule* — we just decline to *enqueue* another run
            // while the prior one is unresolved.
            if !self.store.try_claim_slug(&entry.slug) {
                tracing::info!(
                    slug = %entry.slug,
                    "scheduled_workflow.cron_skip_already_pending"
                );
                continue;
            }
            let Some(definition) = self.store.load_definition(&entry.slug) else {
                tracing::warn!(
                    slug = %entry.slug,
                    "scheduled_workflow.definition_missing_auto_disable"
                );
                self.store.release_slug(&entry.slug);
                // Auto-disable so the Ticker doesn't re-pull this on
                // every minute. Best-effort: a failure here just means
                // the next tick will try again (still cheap — the
                // file scan is `load_definition`, no DB hit).
                if let Err(error) = self.store.set_enabled(entry.slug.clone(), false).await
                {
                    tracing::warn!(
                        slug = %entry.slug,
                        ?error,
                        "scheduled_workflow.auto_disable_failed"
                    );
                }
                continue;
            };
            let task = ScheduledWorkflowTask::new(
                entry.slug.clone(),
                definition,
                Ulid::new().to_string(),
                now,
                self.store.clone(),
            );
            tracing::info!(
                slug = %entry.slug,
                next_run_at = ?entry.next_run_at,
                "scheduled_workflow.dispatch"
            );
            self.scheduler.enqueue(Arc::new(task));
            dispatched += 1;
        }
        Ok(dispatched)
    }

    /// Run a single workflow on demand. Bypasses the schedule (doesn't
    /// touch `next_run_at`) but otherwise reuses the same task path
    /// so "立即运行" produces identical audit rows.
    ///
    /// **Idempotent**: if the slug is already in-flight (cron just
    /// fired, the user double-clicked, or backend has a stale claim
    /// from a previous run that never released), we silently no-op
    /// and return a sentinel run id. From the user's point of view
    /// "run this thing" is the same intent whether or not it's
    /// already running — the row's "正在运行" chip already conveys
    /// the state. The previous design that errored out with
    /// "已经在运行队列中" exposed the internal queue concept and
    /// confused users (Nielsen's "visibility of system status":
    /// don't surface internal data structures the UI doesn't show).
    pub async fn run_now(&self, slug: String) -> crate::error::Result<String> {
        if self.store.load_definition(&slug).is_none() {
            return Err(crate::error::CorivoError::Internal(format!(
                "找不到工作流 `{slug}`"
            )));
        }
        if !self.store.try_claim_slug(&slug) {
            tracing::info!(
                slug = %slug,
                "scheduled_workflow.run_now_already_inflight_noop"
            );
            return Ok(format!("already-inflight-{slug}"));
        }
        // From here on we own the slug claim; if anything fails before
        // the scheduler picks the task up, release it so the next click
        // can re-claim.
        let definition = match self.store.load_definition(&slug) {
            Some(d) => d,
            None => {
                self.store.release_slug(&slug);
                return Err(crate::error::CorivoError::Internal(format!(
                    "找不到工作流 `{slug}`"
                )));
            }
        };
        let run_id = Ulid::new().to_string();
        let task = ScheduledWorkflowTask::new(
            slug.clone(),
            definition,
            run_id.clone(),
            now_utc(),
            self.store.clone(),
        );
        tracing::info!(slug = %slug, run_id = %run_id, "scheduled_workflow.run_now");
        self.scheduler.enqueue(Arc::new(task));
        Ok(run_id)
    }
}
