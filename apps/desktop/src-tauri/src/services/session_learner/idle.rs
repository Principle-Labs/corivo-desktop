//! Idle detector for the session memory learner
//! (memory-system-spec §3.4.2 trigger conditions).
//!
//! Polls `chat_threads` on a fixed tick (default once per minute).
//! Any user thread whose latest message is older than the configured
//! idle threshold AND whose latest message id differs from the
//! learner's last-processed-id gets enqueued.
//!
//! Concurrency model: enqueueing is cheap (the scheduler is the one
//! that throttles to one in-flight task). The poller never blocks —
//! it just submits and moves on. Failure cases (DB unavailable,
//! scheduler not yet up) are best-effort: log and skip the tick.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tauri::Manager;
use tokio::sync::Mutex;

use crate::commands::config::AppState;
use crate::domain::chat::ChatThreadKind;
use crate::services::background_agent_task::{BackgroundAgentScheduler, ScheduleEnqueue, TaskDeps};
use crate::services::session_learner::task::{get_checkpoint, SessionLearnerTask};

#[derive(Clone)]
pub struct IdleHook {
    inner: Arc<HookInner>,
}

struct HookInner {
    app: tauri::AppHandle<tauri::Wry>,
    scheduler: BackgroundAgentScheduler,
    idle_threshold: Duration,
    poll_interval: Duration,
    in_flight: Mutex<HashSet<String>>,
}

impl IdleHook {
    pub fn new(
        app: tauri::AppHandle<tauri::Wry>,
        scheduler: BackgroundAgentScheduler,
        idle_threshold: Duration,
        poll_interval: Duration,
    ) -> Self {
        Self {
            inner: Arc::new(HookInner {
                app,
                scheduler,
                idle_threshold,
                poll_interval,
                in_flight: Mutex::new(HashSet::new()),
            }),
        }
    }

    pub fn start(&self) -> tauri::async_runtime::JoinHandle<()> {
        let inner = self.inner.clone();
        tauri::async_runtime::spawn(async move {
            // First tick after a short warm-up so we don't fight boot.
            tokio::time::sleep(Duration::from_secs(30)).await;
            loop {
                tick(&inner).await;
                tokio::time::sleep(inner.poll_interval).await;
            }
        })
    }
}

async fn tick(inner: &HookInner) {
    let Some(state) = inner.app.try_state::<AppState>() else {
        return;
    };
    let Some(deps) = TaskDeps::from_state(&state, inner.app.clone()).await else {
        return;
    };
    let Ok(threads) = deps.chat_threads.list(100).await else {
        return;
    };
    let now = Utc::now();
    for thread in threads {
        if !matches!(thread.kind, ChatThreadKind::User) {
            continue;
        }
        // updated_at is touched on every user / assistant message; if
        // it's older than the threshold the thread is idle.
        let age = now - thread.updated_at;
        if age.num_seconds() < inner.idle_threshold.as_secs() as i64 {
            continue;
        }
        // Avoid double-queueing the same thread while it's already
        // being processed.
        {
            let in_flight = inner.in_flight.lock().await;
            if in_flight.contains(&thread.id) {
                continue;
            }
        }
        let Ok(latest) = deps.chat_threads.latest_message_id(&thread.id).await else {
            continue;
        };
        let Some(latest_id) = latest else {
            continue;
        };
        let checkpoint = get_checkpoint(&deps, &thread.id).await;
        if checkpoint.as_deref() == Some(latest_id.as_str()) {
            continue;
        }
        let task = SessionLearnerTask::new(thread.id.clone(), checkpoint);
        tracing::info!(
            thread_id = %thread.id,
            after = ?task.after_message_id,
            "session_learner.idle_enqueue"
        );
        // Mark in_flight; the scheduler drops it on completion via a
        // separate path is fine because at worst we double-skip until
        // the next checkpoint advance.
        {
            let mut in_flight = inner.in_flight.lock().await;
            in_flight.insert(thread.id.clone());
        }
        inner.scheduler.enqueue(Arc::new(task));
    }
    // Coarse cleanup: drop in_flight entries whose checkpoint has
    // advanced since they were submitted. That way a successful run
    // unblocks itself the next tick.
    let mut in_flight = inner.in_flight.lock().await;
    let mut to_clear: Vec<String> = Vec::new();
    for thread_id in in_flight.iter() {
        let Ok(Some(latest)) = deps.chat_threads.latest_message_id(thread_id).await else {
            continue;
        };
        let cp = get_checkpoint(&deps, thread_id).await;
        if cp.as_deref() == Some(latest.as_str()) {
            to_clear.push(thread_id.clone());
        }
    }
    for t in to_clear {
        in_flight.remove(&t);
    }
}
