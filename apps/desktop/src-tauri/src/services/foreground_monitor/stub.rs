//! No-op foreground app monitor for non-macOS targets. Corivo is
//! Mac-only today; these exist purely so `cargo build` keeps working
//! on Linux / Windows CI runners.

use std::sync::Arc;

use tauri::AppHandle;
use tokio::sync::mpsc;

use crate::error::Result;
use crate::services::capture_pipeline::event_bus::Event;

pub struct ForegroundAppMonitor;

impl ForegroundAppMonitor {
    pub fn install(_notify_tauri: AppHandle) -> Result<Arc<Self>> {
        Ok(Arc::new(Self))
    }

    pub fn subscribe(
        &self,
        _bus: mpsc::Sender<Event>,
        _pid_hook: Box<dyn Fn(i32) + Send + Sync + 'static>,
    ) {
    }

    pub fn unsubscribe(&self) {}

    pub fn emit_initial_focus(_bus: &mpsc::Sender<Event>) -> Result<()> {
        Ok(())
    }
}
