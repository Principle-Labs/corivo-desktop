//! macOS-specific event sources that fan into the [event bus].
//!
//! Each source owns its OS-level observer (AXObserver + CFRunLoop
//! thread for the active app) and translates raw callbacks into
//! [`Event`] values.
//!
//! Frontmost-app changes are NOT a member of this module — they're
//! observed by [`crate::services::foreground_monitor`], which lives
//! independently of capture state and feeds this bus through the
//! pipeline's `subscribe` call in `start()`.
//!
//! [event bus]: super::event_bus
//! [`Event`]: super::event_bus::Event

#[cfg(target_os = "macos")]
pub mod ax_observer;

#[cfg(target_os = "macos")]
pub use ax_observer::AXObserverThread;

// Non-macOS platforms get inert no-op stubs so the rest of the
// pipeline compiles. Real cross-platform support is out of scope —
// Corivo is Mac-first.
#[cfg(not(target_os = "macos"))]
pub mod stubs;

#[cfg(not(target_os = "macos"))]
pub use stubs::AXObserverThread;
