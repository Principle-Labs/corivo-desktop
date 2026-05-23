//! Event bus for the event-driven capture driver.
//!
//! All event sources (NSWorkspace front-app shifts today; AXObserver
//! notifications, idle detection, manual IPC, safety-net timer in
//! later steps) fan in here through a single `mpsc::Sender<Event>`.
//! `EventCoordinator` (see `coordinator.rs`) is the only consumer.
//!
//! **Why mpsc, not broadcast:** there is exactly one coordinator. Each
//! event needs to be processed exactly once — broadcast semantics
//! would let a slow secondary consumer drop messages silently. mpsc
//! also gives backpressure (`send().await` blocks) for free, which
//! we want when the AX-observer thread floods us with events.

use tokio::sync::mpsc;

/// Channel buffer for events. 128 is sized for a worst-case spike of
/// AX notifications during an SPA route change (a dozen
/// `AXLayoutChanged` + `AXTitleChanged` events fired back to back) +
/// safety net for the coordinator catching up. Drops here would lose
/// triggers; we'd rather backpressure briefly than lose.
pub const EVENT_BUS_CAPACITY: usize = 128;

/// Single union of every wake-up the event-driven driver can receive.
#[derive(Debug, Clone)]
pub enum Event {
    /// NSWorkspace observed the frontmost app change. `to: None`
    /// means "no app identifies itself" (Finder background,
    /// screensaver, etc.) and is skipped by the coordinator.
    AppActivated {
        from: Option<String>,
        to: Option<String>,
    },

    /// AXObserver: focused window inside the active app changed
    /// (kAXFocusedWindowChanged).
    FocusedWindow,

    /// AXObserver: window title changed (kAXTitleChanged). The main
    /// signal for URL / page changes inside browsers and SPAs.
    TitleChanged,

    /// Tauri command (or future IPC) requested an immediate snapshot.
    /// `reason` is opaque, surfaced in tracing only.
    ManualSnapshot { reason: String },

    /// Cooperative shutdown — coordinator drains and exits.
    Shutdown,
}

/// Construct the event bus. Returns the `Sender` for adapters to
/// publish on and the `Receiver` for the coordinator to drain.
pub fn channel() -> (mpsc::Sender<Event>, mpsc::Receiver<Event>) {
    mpsc::channel(EVENT_BUS_CAPACITY)
}
