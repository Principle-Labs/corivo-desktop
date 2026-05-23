//! Daily / startup-on-stale dispatcher for `PersonaDistillTask`
//! (memory-system-spec §4.3 / §11.6).
//!
//! Two trigger conditions:
//!
//! 1. **Startup catch-up.** When the app launches and the existing
//!    `auto-persona.md` is older than 24h (or absent), enqueue one
//!    distill run after a short warm-up.
//! 2. **Daily refresh.** Once we've seen one successful run, sleep
//!    until 03:00 local (configurable later via Config) and trigger
//!    the next.
//!
//! Both go through the shared `BackgroundAgentScheduler`, so they
//! single-flight against session learner runs and any other future
//! tasks.

use std::sync::Arc;
use std::time::Duration;

use tauri::Manager;
use tokio::time::sleep;

use crate::commands::config::AppState;
use crate::services::background_agent_task::{BackgroundAgentScheduler, ScheduleEnqueue};
use crate::services::persona::renderer::AUTO_PERSONA_FILE;
use crate::services::persona::PersonaDistillTask;

const STARTUP_WARMUP: Duration = Duration::from_secs(120);
const DAY: Duration = Duration::from_secs(24 * 60 * 60);
const STALE_THRESHOLD: Duration = Duration::from_secs(24 * 60 * 60);
const DEFAULT_EVIDENCE_WINDOW_DAYS: u32 = 30;

pub fn start(
    app: tauri::AppHandle<tauri::Wry>,
    scheduler: BackgroundAgentScheduler,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        sleep(STARTUP_WARMUP).await;
        // Startup catch-up if needed.
        if persona_is_stale(&app) {
            tracing::info!("persona.startup_enqueue_stale");
            enqueue(&scheduler);
        }
        loop {
            sleep(DAY).await;
            if persona_is_stale(&app) {
                enqueue(&scheduler);
            }
        }
    })
}

fn persona_is_stale(app: &tauri::AppHandle<tauri::Wry>) -> bool {
    let Ok(dir) = app.path().app_data_dir() else {
        return false;
    };
    let path = dir.join(AUTO_PERSONA_FILE);
    let Ok(metadata) = std::fs::metadata(&path) else {
        // Missing file → first-run, definitely stale.
        return true;
    };
    let Ok(modified) = metadata.modified() else {
        return true;
    };
    match std::time::SystemTime::now().duration_since(modified) {
        Ok(age) => age > STALE_THRESHOLD,
        Err(_) => false,
    }
}

fn enqueue(scheduler: &BackgroundAgentScheduler) {
    scheduler.enqueue(Arc::new(PersonaDistillTask::new(
        DEFAULT_EVIDENCE_WINDOW_DAYS,
    )));
}

/// Manual trigger from the Settings UI / dev tooling. Bypasses the
/// staleness check.
pub fn enqueue_now(_app: &tauri::AppHandle<tauri::Wry>, scheduler: &BackgroundAgentScheduler) {
    enqueue(scheduler);
}

/// Tiny shim so `lib.rs` doesn't have to depend on AppState type
/// directly to know whether the schedule loop has started.
pub fn ensure_app_state_available(app: &tauri::AppHandle<tauri::Wry>) -> bool {
    app.try_state::<AppState>().is_some()
}
