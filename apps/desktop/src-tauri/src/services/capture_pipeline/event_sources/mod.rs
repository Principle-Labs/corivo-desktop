//! Helper-backed event sources that fan into the [event bus].
//!
//! Each source owns its helper-side subscription and translates raw
//! helper callbacks into [`Event`] values.
//!
//! Frontmost-app changes are NOT a member of this module — they're
//! observed by [`crate::services::foreground_monitor`], which lives
//! independently of capture state and feeds this bus through the
//! pipeline's `subscribe` call in `start()`.
//!
//! [event bus]: super::event_bus
//! [`Event`]: super::event_bus::Event

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub mod ax_observer;

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub use ax_observer::AXObserverThread;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub mod stubs;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub use stubs::AXObserverThread;
