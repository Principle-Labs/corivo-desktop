//! Double-tap-modifier global hotkey for summoning Quick Ask.
//!
//! macOS' `RegisterEventHotKey` (used by `tauri-plugin-global-shortcut`)
//! refuses to bind a bare modifier — you can only bind a key plus
//! modifiers. Detecting "user just tapped ⌥ twice in <400 ms" therefore
//! has to live one layer up, on top of `NSEvent`'s event monitor for
//! `flagsChanged` events.
//!
//! What we use:
//!   - `NSEvent.addGlobalMonitorForEventsMatchingMask:handler:` —
//!     observes events outside our process. Read-only (we can't
//!     consume the keystroke), no extra permission required.
//!   - `NSEvent.addLocalMonitorForEventsMatchingMask:handler:` —
//!     observes events targeted at our own process. Required because
//!     the global monitor *only* fires for events going to other apps;
//!     when corivo is frontmost (settings page, /ask, anywhere) the
//!     keystroke takes the local responder chain instead. Without
//!     this, double-tap ⌥ goes silent inside the app itself.
//!
//! What we don't use:
//!   - `CGEventTapCreate` — more powerful but needs the user to grant
//!     "Input Monitoring" privacy permission.
//!   - `RegisterEventHotKey` — won't bind modifier-only chords.

#[cfg(target_os = "macos")]
use std::ptr;
#[cfg(target_os = "macos")]
use std::ptr::NonNull;
#[cfg(target_os = "macos")]
use std::sync::{Arc, Mutex};
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
use block2::{DynBlock, RcBlock};
#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2::runtime::AnyObject;
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags};

#[cfg(not(target_os = "macos"))]
use crate::error::Result;
#[cfg(target_os = "macos")]
use crate::error::{CorivoError, Result};

/// Two presses of the modifier within this window count as a "double
/// tap". Tuned to feel like macOS' built-in ⌃⌃ Spotlight gesture
/// (~400 ms tolerated). Anything tighter starts feeling unfair on
/// users who type with Option as part of an IME.
#[cfg(target_os = "macos")]
const DOUBLE_TAP_WINDOW: Duration = Duration::from_millis(400);

/// Owns the registered NSEvent monitor tokens. Drop calls
/// `removeMonitor:` on each so the blocks stop firing after the
/// service exits.
#[cfg(target_os = "macos")]
pub struct DoubleTapHotkey {
    global_monitor: Option<Retained<AnyObject>>,
    local_monitor: Option<Retained<AnyObject>>,
}

/// Non-macOS stub. Quick Ask's double-tap-⌥ summon relies entirely on
/// `NSEvent.addGlobalMonitorForEventsMatchingMask` — there's no
/// portable equivalent in Tauri today, and the corresponding chord on
/// Windows would go through `tauri-plugin-global-shortcut` (different
/// surface) rather than this service. `install` returns a no-op handle
/// so lib.rs's wiring compiles unchanged.
#[cfg(not(target_os = "macos"))]
pub struct DoubleTapHotkey;

#[cfg(not(target_os = "macos"))]
impl DoubleTapHotkey {
    pub fn install<F>(_callback: F) -> Result<Self>
    where
        F: Fn() + Send + Sync + 'static,
    {
        tracing::info!("double_tap_hotkey.skipped (non-macOS)");
        Ok(Self)
    }
}

// SAFETY: monitor tokens are opaque NSObjects the notification center
// owns; reading/dropping them is thread-safe per Apple docs.
#[cfg(target_os = "macos")]
unsafe impl Send for DoubleTapHotkey {}
#[cfg(target_os = "macos")]
unsafe impl Sync for DoubleTapHotkey {}

#[cfg(target_os = "macos")]
#[derive(Debug, Default)]
struct State {
    /// Wall-clock of the previous bare-Option release. Reset to None
    /// after a successful double-tap (so triple-tap doesn't re-trigger
    /// immediately) or whenever we see modifier noise.
    last_release_at: Option<Instant>,
    /// Was the Option flag set in the previous `flagsChanged` event?
    /// We only care about transitions, not absolute state.
    option_was_pressed: bool,
}

#[cfg(target_os = "macos")]
impl DoubleTapHotkey {
    /// Install both a global and a local monitor that call `callback`
    /// whenever the user double-taps Option (⌥). The state machine
    /// is shared, so a tap that the global monitor sees and a tap
    /// that the local monitor sees still count as a single pair.
    /// Callback runs on whatever thread AppKit fires the monitor on
    /// (typically main); keep it non-blocking — spawn a tokio task
    /// inside if you need real work.
    pub fn install<F>(callback: F) -> Result<Self>
    where
        F: Fn() + Send + Sync + 'static,
    {
        let state = Arc::new(Mutex::new(State::default()));
        let cb: Arc<dyn Fn() + Send + Sync> = Arc::new(callback);

        let mask = NSEventMask::FlagsChanged;

        // ---- Global monitor: fires for events going to other apps.
        let state_global = state.clone();
        let cb_global = cb.clone();
        let global_block = RcBlock::new(move |event_ptr: NonNull<NSEvent>| {
            handle_flags_changed(&state_global, &cb_global, event_ptr);
        });
        let global_block_ref: &DynBlock<dyn Fn(NonNull<NSEvent>)> = &global_block;

        let global_monitor =
            NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask, global_block_ref);
        if global_monitor.is_none() {
            return Err(CorivoError::Internal(
                "NSEvent.addGlobalMonitor returned nil".to_string(),
            ));
        }

        // ---- Local monitor: fires for events targeted at our own
        // process. Same state, same callback. The block must return
        // a pointer to keep the event flowing to the responder chain;
        // returning the same pointer we got is the "don't swallow it"
        // convention.
        let state_local = state.clone();
        let cb_local = cb.clone();
        let local_block = RcBlock::new(move |event_ptr: NonNull<NSEvent>| -> *mut NSEvent {
            handle_flags_changed(&state_local, &cb_local, event_ptr);
            event_ptr.as_ptr()
        });
        let local_block_ref: &DynBlock<dyn Fn(NonNull<NSEvent>) -> *mut NSEvent> = &local_block;

        // SAFETY: the block's return value is the unmodified pointer
        // we received from AppKit, which by definition is a valid
        // NSEvent for the duration of the dispatch. That satisfies
        // the binding's "valid pointer or null" precondition.
        let local_monitor =
            unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, local_block_ref) };
        if local_monitor.is_none() {
            // Roll back the global monitor we just installed before
            // surfacing the error — otherwise it leaks and stays
            // armed across retries.
            if let Some(token) = &global_monitor {
                unsafe { NSEvent::removeMonitor(&**token) };
            }
            return Err(CorivoError::Internal(
                "NSEvent.addLocalMonitor returned nil".to_string(),
            ));
        }

        // Silence the "unused for the lifetime of this function" warn —
        // ptr import is what lets us stay future-proof if we ever want
        // to swallow an event by returning ptr::null_mut().
        let _ = ptr::null_mut::<NSEvent>;

        tracing::info!("double_tap_hotkey.installed (option × 2, global + local)");

        Ok(Self {
            global_monitor,
            local_monitor,
        })
    }
}

/// Shared state-machine step driven by both the global and the local
/// monitor. Pulled out as a free function so both monitor blocks can
/// reuse it without each capturing its own copy.
#[cfg(target_os = "macos")]
fn handle_flags_changed(
    state: &Mutex<State>,
    callback: &Arc<dyn Fn() + Send + Sync>,
    event_ptr: NonNull<NSEvent>,
) {
    // SAFETY: AppKit hands us a valid retained NSEvent for the
    // duration of the block call.
    let event = unsafe { event_ptr.as_ref() };
    let flags = event.modifierFlags();
    let option_now = flags.contains(NSEventModifierFlags::Option);

    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    let was = guard.option_was_pressed;
    guard.option_was_pressed = option_now;

    // We only care about Option transitioning from pressed → released.
    // Every other flagsChanged (other modifiers pressed/released,
    // etc) just updates state and exits.
    if !(was && !option_now) {
        return;
    }

    // Reject if any other modifier is also active right at the
    // release moment. "Released Option but Cmd was held" is not a
    // double-tap intent — it's likely a chord that happened to
    // involve Option.
    let other_mods =
        NSEventModifierFlags::Command | NSEventModifierFlags::Control | NSEventModifierFlags::Shift;
    if flags.intersects(other_mods) {
        guard.last_release_at = None;
        return;
    }

    let now = Instant::now();
    let is_double = matches!(
        guard.last_release_at,
        Some(prev) if now.duration_since(prev) < DOUBLE_TAP_WINDOW
    );

    if is_double {
        guard.last_release_at = None;
        drop(guard);
        tracing::debug!("double_tap_hotkey.fired");
        callback();
    } else {
        guard.last_release_at = Some(now);
    }
}

#[cfg(target_os = "macos")]
impl Drop for DoubleTapHotkey {
    fn drop(&mut self) {
        // SAFETY: `removeMonitor:` is thread-safe; the tokens came
        // straight from add{Global,Local}Monitor so they're the right
        // type.
        if let Some(token) = self.global_monitor.take() {
            unsafe { NSEvent::removeMonitor(&*token) };
        }
        if let Some(token) = self.local_monitor.take() {
            unsafe { NSEvent::removeMonitor(&*token) };
        }
        tracing::info!("double_tap_hotkey.uninstalled");
    }
}
