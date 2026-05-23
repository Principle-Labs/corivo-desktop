//! No-op event-source stubs for non-macOS targets. Corivo is Mac-only
//! today; these exist purely so `cargo build` keeps working on Linux
//! / Windows CI runners.

use tokio::sync::mpsc;

use super::super::event_bus::Event;

pub struct AXObserverThread;

impl AXObserverThread {
    pub fn spawn(_bus: mpsc::Sender<Event>) -> Self {
        Self
    }

    pub fn set_target_pid(&self, _pid: i32) {}

    pub fn pid_setter(&self) -> impl Fn(i32) + Send + Sync + 'static {
        |_pid: i32| {}
    }

    pub fn shutdown(self) {}
}
