//! Frontmost-app monitor.
//!
//! Observes `NSWorkspaceDidActivateApplicationNotification` and fans
//! the result out to two sinks:
//!
//!   1. **Quick Ask overlay** — always on. Emits the Tauri event
//!      `capture:focus-activated` to the `quick-ask` webview the
//!      moment AppKit reports a new frontmost app, so the FocusCard
//!      can re-aim immediately. Independent of the capture pipeline,
//!      so the overlay keeps tracking app switches even when capture
//!      is paused.
//!
//!   2. **Capture pipeline (optional)** — pushes
//!      `Event::AppActivated` onto the capture event bus and
//!      retargets the AXObserver thread to the new pid. Wired through
//!      [`ForegroundAppMonitor::subscribe`] when the pipeline starts
//!      and torn down via [`ForegroundAppMonitor::unsubscribe`] when
//!      it stops.
//!
//! Installed once at app boot. The observer token lives for the whole
//! process; `Drop` calls `removeObserver:`.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod stub;

#[cfg(target_os = "macos")]
pub use macos::ForegroundAppMonitor;
#[cfg(not(target_os = "macos"))]
pub use stub::ForegroundAppMonitor;
