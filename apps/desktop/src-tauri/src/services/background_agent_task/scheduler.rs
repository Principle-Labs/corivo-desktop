//! FIFO scheduler with single-flight semantics (memory-system-spec §11.6).
//!
//! Anyone can enqueue a task; the worker pulls one at a time and runs
//! it via `runner::run`. The single-flight guarantee matters because:
//!
//! * The sidecar uses non-trivial LLM credits per task. Two tasks
//!   running in parallel would burn the user's daily quota faster
//!   without finishing any sooner.
//! * The Anthropic rate-limit budget is per-key. Two parallel turns
//!   would 429 each other.
//!
//! Tasks are described as boxed `BackgroundAgentTask` trait objects.
//! The scheduler doesn't know what each one does; it just owns
//! lifecycle (enqueue + serial execution + per-task `consume_output`).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex as StdMutex};

use tauri::AppHandle;
use tokio::sync::oneshot;
use tokio::sync::Mutex;
use tokio::sync::Notify;

use super::{BackgroundAgentTask, TaskDeps};
use crate::commands::config::AppState;

pub type TaskRef = Arc<dyn BackgroundAgentTask>;

/// Hard cap on `deps_not_ready_requeue` retries before giving up on a
/// task. With the 60 s sleep between retries this works out to ~5
/// minutes of patience — enough for boot-time login / model_catalog
/// fetch to settle, but not "forever" the way the old loop went
/// (real incident 2026-05-22: persona_distill requeued for ~4 hours
/// while the user was logged out, blocking the worker).
const MAX_DEPS_RETRIES: u32 = 5;

/// Tracks per-attempt state alongside the task itself. Living in the
/// queue (rather than a side-table keyed on `*const dyn`) keeps the
/// state local to the worker loop and dies naturally when the task
/// is dropped.
struct QueuedTask {
    task: TaskRef,
    /// How many times we've popped this task and bounced off
    /// `deps_not_ready`. Reset by a successful dispatch (the task
    /// leaves the queue).
    deps_attempts: u32,
}

/// External handle the rest of the app uses to enqueue.
pub trait ScheduleEnqueue: Send + Sync {
    fn enqueue(&self, task: TaskRef);
}

#[derive(Clone)]
pub struct BackgroundAgentScheduler {
    inner: Arc<SchedulerInner>,
}

struct SchedulerInner {
    queue: Mutex<VecDeque<QueuedTask>>,
    notify: Notify,
    app: AppHandle<tauri::Wry>,
    /// Cancellation channels for currently-running cancellable
    /// tasks, keyed by `cancellation_key()`. Only tasks whose
    /// `cancellation_key()` returns `Some(_)` are tracked here;
    /// session_learner / persona_distill etc. stay invisible to the
    /// cancel surface.
    ///
    /// **Why oneshot, not AbortHandle**: the first cut used
    /// `tokio::spawn` + `AbortHandle`, but spawning a child task
    /// from inside Tauri's async_runtime context caused
    /// `Cannot block the current thread from within a runtime`
    /// panics — Tauri's runtime + a nested tokio::spawn don't mix
    /// cleanly for tasks that touch tauri-plugin-notification
    /// internals. Oneshot keeps the runner on the worker's own
    /// task; cancel just trips `select!` which drops the runner
    /// future. `kill_on_drop(true)` on the sidecar child still
    /// guarantees SIGKILL.
    cancellable: StdMutex<HashMap<String, oneshot::Sender<()>>>,
}

impl BackgroundAgentScheduler {
    pub fn new(app: AppHandle<tauri::Wry>) -> Self {
        Self {
            inner: Arc::new(SchedulerInner {
                queue: Mutex::new(VecDeque::new()),
                notify: Notify::new(),
                app,
                cancellable: StdMutex::new(HashMap::new()),
            }),
        }
    }

    /// Cancel a running OR queued cancellable task by its
    /// `cancellation_key`. Returns `true` if anything was actually
    /// stopped — false if no task with that key was found.
    ///
    /// Running task: abort the spawned future. `kill_on_drop(true)`
    /// on the sidecar child kills the process immediately;
    /// `runner::run`'s caller sees `JoinError::Cancelled` and routes
    /// to the same `on_dispatch_aborted` cleanup as an
    /// `Err`-returning runner.
    ///
    /// Queued task (not yet dispatched): remove from queue + call
    /// `on_dispatch_aborted` directly so the task can release any
    /// dedup claim + emit a `workflow:completed` failure event. The
    /// user's intent is "stop this thing", so the queued case must
    /// not silently linger waiting to fire later.
    pub async fn cancel(&self, key: &str) -> bool {
        // Running case — fire the cancellation oneshot. The runner's
        // `select!` arm picks it up + drops the runner future;
        // `kill_on_drop(true)` on the sidecar takes care of SIGKILL.
        let cancelled_running = {
            let mut map = match self.inner.cancellable.lock() {
                Ok(m) => m,
                Err(poisoned) => poisoned.into_inner(),
            };
            map.remove(key)
        };
        if let Some(sender) = cancelled_running {
            // `send(())` only fails if the receiver was already
            // dropped (runner finished normally between us removing
            // from the map + sending). Either way we did our part.
            let _ = sender.send(());
            tracing::info!(key, "background_agent_task.cancel_signalled_running");
            return true;
        }

        // Queued case — pull every matching entry out of the queue
        // and notify them. Multiple matches are rare (workflow store
        // dedup catches most) but we handle them for safety.
        let mut queue = self.inner.queue.lock().await;
        let to_cancel: Vec<TaskRef> = queue
            .iter()
            .filter(|q| q.task.cancellation_key().as_deref() == Some(key))
            .map(|q| q.task.clone())
            .collect();
        if to_cancel.is_empty() {
            return false;
        }
        queue.retain(|q| q.task.cancellation_key().as_deref() != Some(key));
        drop(queue);
        for task in to_cancel {
            task.on_dispatch_aborted("cancelled_by_user");
        }
        tracing::info!(key, "background_agent_task.cancel_removed_queued");
        true
    }

    /// Spawn the worker loop. Safe to call from Tauri's `setup`
    /// closure — uses `tauri::async_runtime::spawn`, which enters the
    /// Tauri-owned tokio runtime regardless of the caller's context.
    pub fn start(&self) -> tauri::async_runtime::JoinHandle<()> {
        let inner = self.inner.clone();
        tauri::async_runtime::spawn(async move {
            run_worker_loop(inner).await;
        })
    }
}

impl ScheduleEnqueue for BackgroundAgentScheduler {
    fn enqueue(&self, task: TaskRef) {
        let inner = self.inner.clone();
        // `tauri::async_runtime::spawn` enters the Tauri-managed tokio
        // runtime, so this is safe from both sync (Tauri setup, Tauri
        // command handlers before the first await) and async contexts.
        tauri::async_runtime::spawn(async move {
            let mut queue = inner.queue.lock().await;
            tracing::info!(
                task = task.kind().as_str(),
                queue_len = queue.len() + 1,
                "background_agent_task.enqueued"
            );
            queue.push_back(QueuedTask {
                task,
                deps_attempts: 0,
            });
            inner.notify.notify_one();
        });
    }
}

async fn run_worker_loop(inner: Arc<SchedulerInner>) {
    loop {
        let next = {
            let mut queue = inner.queue.lock().await;
            queue.pop_front()
        };
        let Some(mut queued) = next else {
            // Empty queue — wait for an enqueue notification.
            inner.notify.notified().await;
            continue;
        };

        let kind = queued.task.kind();
        tracing::info!(task = kind.as_str(), "background_agent_task.dispatch");

        // Lazily build TaskDeps each iteration so a mid-session login /
        // model switch is picked up on the next run.
        let state_opt = inner.app.try_state::<AppState>();
        let Some(state) = state_opt else {
            tracing::warn!(
                task = kind.as_str(),
                "background_agent_task.appstate_unavailable_skip"
            );
            continue;
        };
        let Some(deps) = TaskDeps::from_state(&state, inner.app.clone()).await else {
            queued.deps_attempts += 1;
            if queued.deps_attempts >= MAX_DEPS_RETRIES {
                tracing::warn!(
                    task = kind.as_str(),
                    attempts = queued.deps_attempts,
                    "background_agent_task.deps_not_ready_abandon"
                );
                // Give up. Notify the task so it can release any
                // dedup claim, persist a failure row, fire a
                // workflow:completed event with a clear reason, etc.
                queued
                    .task
                    .on_dispatch_aborted("deps_not_ready_exceeded_retries");
                continue;
            }
            tracing::info!(
                task = kind.as_str(),
                attempts = queued.deps_attempts,
                "background_agent_task.deps_not_ready_requeue"
            );
            // Push back to the front so we don't starve other tasks.
            inner.queue.lock().await.push_front(queued);
            // Sleep a beat so we don't busy-loop on a slow boot.
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            continue;
        };

        // Cancellable tasks (workflows): race the runner against a
        // oneshot cancellation signal. When the IPC sends on the
        // oneshot, `select!` resolves to the cancel arm which drops
        // the runner future; the sidecar dies via `kill_on_drop`.
        // Non-cancellable tasks (session_learner / persona) just
        // await the runner inline.
        //
        // No `tokio::spawn` here on purpose — nesting a fresh spawn
        // inside Tauri's async_runtime context panicked with
        // "Cannot block the current thread from within a runtime"
        // the moment the inner future touched anything that resolved
        // through tauri-plugin-notification.
        let cancellation_key = queued.task.cancellation_key();
        let result: DispatchResult = if let Some(key) = cancellation_key.clone() {
            let (tx, rx) = oneshot::channel::<()>();
            {
                let mut map = match inner.cancellable.lock() {
                    Ok(m) => m,
                    Err(p) => p.into_inner(),
                };
                map.insert(key.clone(), tx);
            }
            let outcome = tokio::select! {
                res = super::runner::run(&*queued.task, &deps) => DispatchOutcome::Ran(res),
                _ = rx => DispatchOutcome::Cancelled,
            };
            {
                let mut map = match inner.cancellable.lock() {
                    Ok(m) => m,
                    Err(p) => p.into_inner(),
                };
                map.remove(&key);
            }
            match outcome {
                DispatchOutcome::Ran(Ok(o)) => DispatchResult::Done(o),
                DispatchOutcome::Ran(Err(e)) => DispatchResult::RunnerErr(e),
                DispatchOutcome::Cancelled => DispatchResult::Cancelled,
            }
        } else {
            match super::runner::run(&*queued.task, &deps).await {
                Ok(o) => DispatchResult::Done(o),
                Err(e) => DispatchResult::RunnerErr(e),
            }
        };

        match result {
            DispatchResult::Done(outcome) => {
                tracing::info!(
                    task = kind.as_str(),
                    thread_id = %outcome.thread_id,
                    success = outcome.success,
                    "background_agent_task.completed"
                );
            }
            DispatchResult::RunnerErr(error) => {
                tracing::warn!(
                    task = kind.as_str(),
                    ?error,
                    "background_agent_task.unexpected_error"
                );
                queued
                    .task
                    .on_dispatch_aborted(&format!("runner_error: {error}"));
            }
            DispatchResult::Cancelled => {
                tracing::info!(
                    task = kind.as_str(),
                    "background_agent_task.cancelled_by_user"
                );
                queued.task.on_dispatch_aborted("cancelled_by_user");
            }
        }
    }
}

/// Intermediate value while we still own the `&deps` borrow inside
/// the select!. Once we've unblocked the borrow we collapse this
/// into [`DispatchResult`] for the outer match.
enum DispatchOutcome {
    Ran(crate::error::Result<super::runner::TaskOutcome>),
    Cancelled,
}

/// Terminal state of one dispatch attempt; drives which
/// `on_dispatch_aborted` reason text we route to (if any).
enum DispatchResult {
    Done(super::runner::TaskOutcome),
    RunnerErr(crate::error::CorivoError),
    Cancelled,
}

// Pulled from AppState via the worker — re-exported here so the
// `try_state` lookup compiles even if AppState moves modules later.
use tauri::Manager;
