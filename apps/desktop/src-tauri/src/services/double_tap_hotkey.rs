//! Double-tap-modifier global hotkey for summoning Quick Ask.
//!
//! `RegisterEventHotKey` (used by `tauri-plugin-global-shortcut`)
//! refuses to bind a bare modifier — you can only bind a key plus
//! modifiers. Detecting "user just tapped the platform modifier twice
//! in <400 ms" therefore has to live one layer up: `NSEvent`'
//! `flagsChanged` events on macOS, and a low-level keyboard hook on
//! Windows.
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

#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::ptr;
#[cfg(target_os = "macos")]
use std::ptr::NonNull;
#[cfg(target_os = "windows")]
use std::sync::OnceLock;
#[cfg(target_os = "windows")]
use std::sync::{mpsc, Arc, Mutex};
#[cfg(target_os = "macos")]
use std::sync::{Arc, Mutex};
#[cfg(target_os = "windows")]
use std::thread::{self, JoinHandle};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
use block2::{DynBlock, RcBlock};
#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2::runtime::AnyObject;
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags};
#[cfg(target_os = "windows")]
use windows_sys::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    System::Threading::GetCurrentThreadId,
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, VK_CONTROL, VK_LMENU, VK_LWIN, VK_MENU, VK_RMENU, VK_RWIN, VK_SHIFT,
        },
        WindowsAndMessaging::{
            CallNextHookEx, GetMessageW, PeekMessageW, PostThreadMessageW, SetWindowsHookExW,
            UnhookWindowsHookEx, KBDLLHOOKSTRUCT, MSG, PM_NOREMOVE, WH_KEYBOARD_LL, WM_APP,
            WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
        },
    },
};

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
use crate::error::Result;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use crate::error::{CorivoError, Result};

/// Two presses of the modifier within this window count as a "double
/// tap". Tuned to feel like macOS' built-in ⌃⌃ Spotlight gesture
/// (~400 ms tolerated). Anything tighter starts feeling unfair on
/// users who type with Option/Alt as part of normal input.
#[cfg(any(target_os = "macos", target_os = "windows"))]
const DOUBLE_TAP_WINDOW: Duration = Duration::from_millis(400);

/// Owns the registered NSEvent monitor tokens. Drop calls
/// `removeMonitor:` on each so the blocks stop firing after the
/// service exits.
#[cfg(target_os = "macos")]
pub struct DoubleTapHotkey {
    global_monitor: Option<Retained<AnyObject>>,
    local_monitor: Option<Retained<AnyObject>>,
}

/// Windows owns a background thread with a message loop. Low-level
/// keyboard hooks are delivered to the installing thread, so the hook
/// cannot be installed on a transient setup stack frame.
#[cfg(target_os = "windows")]
pub struct DoubleTapHotkey {
    thread_id: u32,
    join: Option<JoinHandle<()>>,
}

// SAFETY: monitor tokens are opaque NSObjects the notification center
// owns; reading/dropping them is thread-safe per Apple docs.
#[cfg(target_os = "macos")]
unsafe impl Send for DoubleTapHotkey {}
#[cfg(target_os = "macos")]
unsafe impl Sync for DoubleTapHotkey {}
#[cfg(target_os = "windows")]
unsafe impl Send for DoubleTapHotkey {}
#[cfg(target_os = "windows")]
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

#[cfg(target_os = "windows")]
#[derive(Debug, Default)]
struct State {
    last_release_at: Option<Instant>,
    alt_was_pressed: bool,
    chord_dirty: bool,
}

#[cfg(target_os = "windows")]
struct WindowsHookState {
    state: State,
    callback: Arc<dyn Fn() + Send + Sync>,
}

#[cfg(target_os = "windows")]
static WINDOWS_HOOK_STATE: OnceLock<Mutex<Option<WindowsHookState>>> = OnceLock::new();

#[cfg(target_os = "windows")]
const STOP_HOTKEY_THREAD: u32 = WM_APP + 0x4c;

/// Non-desktop/non-supported stub. Mobile builds compile the service
/// graph, but Quick Ask's global bare-modifier listener is desktop-only.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub struct DoubleTapHotkey;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl DoubleTapHotkey {
    pub fn install<F>(_callback: F) -> Result<Self>
    where
        F: Fn() + Send + Sync + 'static,
    {
        tracing::info!("double_tap_hotkey.skipped (unsupported platform)");
        Ok(Self)
    }
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

#[cfg(target_os = "windows")]
impl DoubleTapHotkey {
    /// Install a low-level keyboard hook that calls `callback` whenever
    /// the user double-taps Alt. We intentionally do not consume the
    /// Alt events, so normal Windows menu/focus behavior remains intact.
    pub fn install<F>(callback: F) -> Result<Self>
    where
        F: Fn() + Send + Sync + 'static,
    {
        let slot = windows_hook_slot();
        {
            let mut guard = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if guard.is_some() {
                return Err(CorivoError::Internal(
                    "Windows Alt hotkey hook is already installed".to_string(),
                ));
            }
            *guard = Some(WindowsHookState {
                state: State::default(),
                callback: Arc::new(callback),
            });
        }

        let (ready_tx, ready_rx) = mpsc::channel();
        let join = thread::Builder::new()
            .name("corivo-alt-hotkey".to_string())
            .spawn(move || run_windows_hotkey_thread(ready_tx))
            .map_err(|error| {
                clear_windows_hook_state();
                CorivoError::Internal(format!("spawn Windows Alt hotkey thread: {error}"))
            })?;

        match ready_rx.recv() {
            Ok(Ok(thread_id)) => {
                tracing::info!("double_tap_hotkey.installed (alt x 2, windows hook)");
                Ok(Self {
                    thread_id,
                    join: Some(join),
                })
            }
            Ok(Err(error)) => {
                let _ = join.join();
                Err(CorivoError::Internal(error))
            }
            Err(error) => {
                let _ = join.join();
                Err(CorivoError::Internal(format!(
                    "Windows Alt hotkey thread exited before ready: {error}"
                )))
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn windows_hook_slot() -> &'static Mutex<Option<WindowsHookState>> {
    WINDOWS_HOOK_STATE.get_or_init(|| Mutex::new(None))
}

#[cfg(target_os = "windows")]
fn clear_windows_hook_state() {
    let mut guard = windows_hook_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = None;
}

#[cfg(target_os = "windows")]
fn run_windows_hotkey_thread(ready_tx: mpsc::Sender<std::result::Result<u32, String>>) {
    unsafe {
        // Force this thread's message queue to exist before we publish
        // the id. Drop uses PostThreadMessageW to ask the loop to stop.
        let mut msg: MSG = std::mem::zeroed();
        let _ = PeekMessageW(&mut msg, ptr::null_mut(), 0, 0, PM_NOREMOVE);

        let hook = SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(windows_keyboard_proc),
            ptr::null_mut(),
            0,
        );
        if hook.is_null() {
            let error = std::io::Error::last_os_error();
            clear_windows_hook_state();
            let _ = ready_tx.send(Err(format!("SetWindowsHookExW(WH_KEYBOARD_LL): {error}")));
            return;
        }

        let thread_id = GetCurrentThreadId();
        let _ = ready_tx.send(Ok(thread_id));

        loop {
            let result = GetMessageW(&mut msg, ptr::null_mut(), 0, 0);
            if result <= 0 || msg.message == STOP_HOTKEY_THREAD {
                break;
            }
        }

        if UnhookWindowsHookEx(hook) == 0 {
            tracing::warn!(
                error = %std::io::Error::last_os_error(),
                "double_tap_hotkey.windows_unhook_failed"
            );
        }
        clear_windows_hook_state();
        tracing::info!("double_tap_hotkey.uninstalled (windows)");
    }
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn windows_keyboard_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 {
        let event = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        handle_windows_key_event(wparam as u32, event);
    }
    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}

#[cfg(target_os = "windows")]
fn handle_windows_key_event(message: u32, event: &KBDLLHOOKSTRUCT) {
    let is_key_down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;
    let is_key_up = message == WM_KEYUP || message == WM_SYSKEYUP;
    if !is_key_down && !is_key_up {
        return;
    }

    let is_alt = [VK_MENU, VK_LMENU, VK_RMENU]
        .iter()
        .any(|key| event.vkCode == u32::from(*key));
    let slot = windows_hook_slot();
    let mut guard = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(hook_state) = guard.as_mut() else {
        return;
    };

    if is_alt {
        if is_key_down {
            hook_state.state.alt_was_pressed = true;
            return;
        }

        if is_key_up && hook_state.state.alt_was_pressed {
            hook_state.state.alt_was_pressed = false;
            if hook_state.state.chord_dirty || other_windows_modifier_is_down() {
                hook_state.state.chord_dirty = false;
                hook_state.state.last_release_at = None;
                return;
            }
            let fired = handle_modifier_release(&mut hook_state.state);
            let callback = fired.then(|| hook_state.callback.clone());
            drop(guard);
            if let Some(callback) = callback {
                tracing::debug!("double_tap_hotkey.fired (windows)");
                callback();
            }
        }
        return;
    }

    if is_key_down && hook_state.state.alt_was_pressed {
        hook_state.state.chord_dirty = true;
        hook_state.state.last_release_at = None;
    }
}

#[cfg(target_os = "windows")]
fn handle_modifier_release(state: &mut State) -> bool {
    state.chord_dirty = false;
    let now = Instant::now();
    let is_double = matches!(
        state.last_release_at,
        Some(prev) if now.duration_since(prev) < DOUBLE_TAP_WINDOW
    );

    if is_double {
        state.last_release_at = None;
        true
    } else {
        state.last_release_at = Some(now);
        false
    }
}

#[cfg(target_os = "windows")]
fn other_windows_modifier_is_down() -> bool {
    [VK_CONTROL, VK_SHIFT, VK_LWIN, VK_RWIN]
        .iter()
        .any(|key| unsafe { (GetAsyncKeyState(i32::from(*key)) as u16 & 0x8000) != 0 })
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

#[cfg(target_os = "windows")]
impl Drop for DoubleTapHotkey {
    fn drop(&mut self) {
        unsafe {
            if PostThreadMessageW(self.thread_id, STOP_HOTKEY_THREAD, 0, 0) == 0 {
                tracing::warn!(
                    error = %std::io::Error::last_os_error(),
                    "double_tap_hotkey.windows_stop_post_failed"
                );
            }
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
