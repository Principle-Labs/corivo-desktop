//! Notification-overlay window (v1700).
//!
//! Right-corner toast window that surfaces scheduled-workflow run
//! completions. Modeled on `quick_ask_window.rs`:
//!
//! * Declared in `tauri.conf.json` as `visible: false`.
//! * Booted once in `lib.rs` via `apply_notification_overlay_window_mode`
//!   to install the macOS panel mode (Floating level, NonactivatingPanel
//!   style, CanJoinAllSpaces collection behavior).
//! * Positioned at the active screen's top-right by
//!   [`position_top_right`] before each show.
//!
//! Unlike Quick Ask, this window:
//!
//! * MUST NOT become the key window — focus stays in whatever app the
//!   user was just in.
//! * Has no hotkey or input; it just renders one toast at a time and
//!   accepts a click to open the run's chat thread.

#[cfg(target_os = "macos")]
use tauri::Manager;
use tauri::{LogicalPosition, LogicalSize, Runtime, WebviewWindow};

#[cfg(target_os = "macos")]
use tauri_nspanel::{
    tauri_panel, CollectionBehavior, ManagerExt, PanelLevel, StyleMask, WebviewWindowExt,
};

#[cfg(target_os = "macos")]
tauri_panel! {
    panel!(NotificationOverlayPanel {
        config: {
            // We never want to become key — the user's focus must stay
            // in their previous app. The panel is read-and-click, no
            // keyboard input.
            can_become_key_window: false,
            is_floating_panel: true,
        }
    })
}

/// Default size of the overlay window (logical pixels). Mirrors the
/// `width`/`height` declared in `tauri.conf.json`.
pub const NOTIFICATION_OVERLAY_WIDTH: f64 = 360.0;
pub const NOTIFICATION_OVERLAY_HEIGHT: f64 = 96.0;

/// Margin from the screen edges (logical pixels). Top margin clears
/// the macOS menu bar with comfortable breathing room; right margin
/// matches.
const SCREEN_MARGIN: f64 = 16.0;
const MAC_MENU_BAR_HEIGHT: f64 = 28.0;

/// Convert the overlay window to a Spotlight-style NSPanel and install
/// the level / style / collection-behavior that lets it float above
/// other apps' fullscreen Spaces without ever stealing focus.
///
/// Idempotent — `to_panel()` swizzles the underlying NSWindow class,
/// so calling it twice would re-swizzle. The function bails out early
/// if the panel has already been registered with the plugin's manager.
///
/// MUST run on the macOS main thread.
pub fn apply_notification_overlay_window_mode<R: Runtime>(window: &WebviewWindow<R>) {
    #[cfg(target_os = "macos")]
    {
        let app = window.app_handle();
        let label = window.label();

        if app.get_webview_panel(label).is_ok() {
            return;
        }

        let panel = match window.to_panel::<NotificationOverlayPanel<R>>() {
            Ok(p) => p,
            Err(error) => {
                tracing::warn!(?error, "notification_overlay.window.to_panel_failed");
                return;
            }
        };

        // Floating level — same as Quick Ask. Clears regular windows
        // without sitting above the IME candidate panel.
        panel.set_level(PanelLevel::Floating.value());

        // Borderless + NonactivatingPanel: no chrome, and the panel
        // CANNOT become key (so showing it doesn't pull Corivo to the
        // foreground or steal focus from whatever the user is doing).
        panel.set_style_mask(StyleMask::empty().borderless().nonactivating_panel().into());

        // Spotlight-style: visible across every Space (including other
        // apps' fullscreen Spaces) and excluded from Cmd+Tab cycling.
        panel.set_collection_behavior(
            CollectionBehavior::new()
                .can_join_all_spaces()
                .full_screen_auxiliary()
                .stationary()
                .into(),
        );

        // Hiding the panel on app deactivate would defeat the purpose
        // — the user is supposed to be looking at someone else's app
        // when these fire.
        panel.set_hides_on_deactivate(false);
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
    }
}

/// Place the window at the top-right corner of the monitor that
/// currently contains the cursor. Falls back to the primary monitor
/// when the cursor position is unavailable (e.g. no mouse moved yet
/// since boot).
///
/// MUST run on the macOS main thread.
pub fn position_top_right<R: Runtime>(window: &WebviewWindow<R>) {
    let app = window.app_handle();
    let monitor = pick_monitor(window).or_else(|| app.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        tracing::warn!("notification_overlay.position.no_monitor");
        return;
    };

    let scale = monitor.scale_factor().max(1.0);
    let mon_pos = monitor.position();
    let mon_size = monitor.size();
    // Tauri returns physical pixels here; LogicalPosition is what we
    // hand to set_position, so divide by the scale factor.
    let mon_logical_x = (mon_pos.x as f64) / scale;
    let mon_logical_y = (mon_pos.y as f64) / scale;
    let mon_logical_w = (mon_size.width as f64) / scale;

    let top_offset = if cfg!(target_os = "macos") {
        MAC_MENU_BAR_HEIGHT + SCREEN_MARGIN
    } else {
        SCREEN_MARGIN
    };

    let x = mon_logical_x + mon_logical_w - NOTIFICATION_OVERLAY_WIDTH - SCREEN_MARGIN;
    let y = mon_logical_y + top_offset;
    if let Err(error) = window.set_position(LogicalPosition::new(x, y)) {
        tracing::warn!(?error, "notification_overlay.position.set_failed");
    }
    // Reset to the declared size so a previously-resized toast doesn't
    // leak its dimensions into the next event.
    if let Err(error) = window.set_size(LogicalSize::new(
        NOTIFICATION_OVERLAY_WIDTH,
        NOTIFICATION_OVERLAY_HEIGHT,
    )) {
        tracing::warn!(?error, "notification_overlay.position.resize_failed");
    }
}

/// Choose the monitor that contains the cursor, when we can resolve
/// one. Returns `None` if the cursor query fails or no monitor
/// rectangle contains the point — the caller falls back to the primary.
fn pick_monitor<R: Runtime>(window: &WebviewWindow<R>) -> Option<tauri::Monitor> {
    let app = window.app_handle();
    let cursor = app.cursor_position().ok()?;
    let monitors = app.available_monitors().ok()?;
    monitors.into_iter().find(|m| {
        let pos = m.position();
        let size = m.size();
        let x0 = pos.x as f64;
        let y0 = pos.y as f64;
        let x1 = x0 + size.width as f64;
        let y1 = y0 + size.height as f64;
        cursor.x >= x0 && cursor.x < x1 && cursor.y >= y0 && cursor.y < y1
    })
}

/// Show the overlay window. On macOS this goes through the panel
/// handle so `orderFront:` happens via the swizzled class.
///
/// MUST run on the macOS main thread.
pub fn show_overlay<R: Runtime>(window: &WebviewWindow<R>) {
    #[cfg(target_os = "macos")]
    {
        let app = window.app_handle();
        let label = window.label();
        if let Ok(panel) = app.get_webview_panel(label) {
            // `panel.show()` calls `orderFrontRegardless` under the
            // hood — visible across spaces, doesn't key the window
            // (we already set `can_become_key_window: false` above).
            panel.show();
            return;
        }
    }
    let _ = window.show();
}

/// Hide the overlay window. On macOS this `orderOut:`s the NSPanel.
///
/// MUST run on the macOS main thread.
pub fn hide_overlay<R: Runtime>(window: &WebviewWindow<R>) {
    #[cfg(target_os = "macos")]
    {
        let app = window.app_handle();
        let label = window.label();
        if let Ok(panel) = app.get_webview_panel(label) {
            panel.hide();
            return;
        }
    }
    let _ = window.hide();
}
