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

use std::collections::VecDeque;
use std::sync::Arc;

use tauri::AppHandle;
use tokio::sync::Mutex;
use tokio::sync::Notify;

use super::{BackgroundAgentTask, TaskDeps};
use crate::commands::config::AppState;

pub type TaskRef = Arc<dyn BackgroundAgentTask>;

/// External handle the rest of the app uses to enqueue.
pub trait ScheduleEnqueue: Send + Sync {
    fn enqueue(&self, task: TaskRef);
}

#[derive(Clone)]
pub struct BackgroundAgentScheduler {
    inner: Arc<SchedulerInner>,
}

struct SchedulerInner {
    queue: Mutex<VecDeque<TaskRef>>,
    notify: Notify,
    app: AppHandle<tauri::Wry>,
}

impl BackgroundAgentScheduler {
    pub fn new(app: AppHandle<tauri::Wry>) -> Self {
        Self {
            inner: Arc::new(SchedulerInner {
                queue: Mutex::new(VecDeque::new()),
                notify: Notify::new(),
                app,
            }),
        }
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
            queue.push_back(task);
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
        let Some(task) = next else {
            // Empty queue — wait for an enqueue notification.
            inner.notify.notified().await;
            continue;
        };

        let kind = task.kind();
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
            tracing::info!(
                task = kind.as_str(),
                "background_agent_task.deps_not_ready_requeue"
            );
            // Push back to the front so we don't starve other tasks.
            inner.queue.lock().await.push_front(task);
            // Sleep a beat so we don't busy-loop on a slow boot.
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            continue;
        };

        match super::runner::run(&*task, &deps).await {
            Ok(outcome) => {
                tracing::info!(
                    task = kind.as_str(),
                    thread_id = %outcome.thread_id,
                    success = outcome.success,
                    "background_agent_task.completed"
                );
            }
            Err(error) => {
                tracing::warn!(
                    task = kind.as_str(),
                    ?error,
                    "background_agent_task.unexpected_error"
                );
            }
        }
    }
}

// Pulled from AppState via the worker — re-exported here so the
// `try_state` lookup compiles even if AppState moves modules later.
use tauri::Manager;
