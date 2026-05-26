//! Frontmost-app monitor.
//!
//! Observes helper-emitted foreground activation events and fans the
//! result out to two sinks:
//!
//!   1. **Quick Ask overlay** — always on. Emits the Tauri event
//!      `capture:focus-activated` to the `quick-ask` webview the
//!      moment the platform helper reports a new frontmost app, so the
//!      FocusCard can re-aim immediately. Independent of the capture
//!      pipeline, so the overlay keeps tracking app switches even when
//!      capture is paused.
//!
//!   2. **Capture pipeline (optional)** — pushes
//!      `Event::AppActivated` onto the capture event bus and
//!      retargets the AXObserver thread to the new pid. Wired through
//!      [`ForegroundAppMonitor::subscribe`] when the pipeline starts
//!      and torn down via [`ForegroundAppMonitor::unsubscribe`] when
//!      it stops.
//!
//! Installed once at app boot. `Drop` unsubscribes from the helper.

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod helper;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod stub;

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub use helper::ForegroundAppMonitor;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub use stub::ForegroundAppMonitor;
