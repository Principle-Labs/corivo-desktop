#[cfg(target_os = "macos")]
use tauri::Manager;
use tauri::{Runtime, WebviewWindow};

#[cfg(target_os = "macos")]
use tauri_nspanel::{
    tauri_panel, CollectionBehavior, ManagerExt, PanelLevel, StyleMask, WebviewWindowExt,
};

#[cfg(target_os = "macos")]
tauri_panel! {
    panel!(QuickAskPanel {
        config: {
            // NonactivatingPanel-style panels return NO from
            // canBecomeKeyWindow by default (no titled/resizable mask),
            // which would make `makeKeyWindow` a no-op and the user
            // could not type into the input. Force it to YES.
            can_become_key_window: true,
            is_floating_panel: true,
        }
    })
}

/// Convert the quick-ask window into a Spotlight-style NSPanel and
/// install the level / style / collection-behavior that lets it float
/// above other apps' fullscreen spaces and accept keyboard input
/// without pulling Corivo to the foreground.
///
/// Idempotent: `to_panel()` swizzles the underlying NSWindow class, so
/// calling it twice on the same window would re-swizzle a class that
/// is already our subclass. The function bails out early if the panel
/// has already been registered with the plugin's manager.
///
/// MUST run on the macOS main thread (`run_on_main` in lib.rs).
pub fn apply_quick_ask_overlay_window_mode<R: Runtime>(window: &WebviewWindow<R>) {
    #[cfg(target_os = "macos")]
    {
        let app = window.app_handle();
        let label = window.label();

        if app.get_webview_panel(label).is_ok() {
            return;
        }

        let panel = match window.to_panel::<QuickAskPanel<R>>() {
            Ok(p) => p,
            Err(error) => {
                tracing::warn!(?error, "quick_ask.window.to_panel_failed");
                return;
            }
        };

        // Floating level (4) — the standard "always above normal
        // windows, below most system overlays" level used by tool
        // palettes and color pickers. We picked it specifically to
        // sit below every IME candidate window we know of:
        //   - macOS built-in 中文输入法 on macOS 26 draws its
        //     candidate panel at a level somewhere between Floating
        //     and MainMenu (Status 25 covered it; MainMenu 24 still
        //     covered it — CO-50 follow-up). Floating (4) clears it.
        //   - Third-party IMEs (Sogou / RIME / WeChat) sit at
        //     Status (25) or higher and were already fine.
        // Cross-Space / over-fullscreen visibility comes from the
        // CanJoinAllSpaces + FullScreenAuxiliary collection behavior
        // below, NOT from the level — so dropping the level is safe.
        panel.set_level(PanelLevel::Floating.value());

        // Borderless + NonactivatingPanel: no chrome, and the panel
        // can become key window without bringing Corivo to the
        // foreground. This is the bit that lets the user type
        // immediately on summon and lets focus return to the previous
        // app on dismiss — without it, `makeKeyWindow` would activate
        // Corivo and steal the dock spot.
        panel.set_style_mask(StyleMask::empty().borderless().nonactivating_panel().into());

        // Spotlight-style: visible across every Space (including other
        // apps' fullscreen Spaces) and not part of Cmd+Tab cycling.
        panel.set_collection_behavior(
            CollectionBehavior::new()
                .can_join_all_spaces()
                .full_screen_auxiliary()
                .stationary()
                .into(),
        );

        // We dismiss the panel explicitly (Esc / hotkey toggle / send).
        // Don't let AppKit hide it just because Corivo lost focus —
        // the user is supposed to be looking at someone else's app.
        panel.set_hides_on_deactivate(false);
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
    }
}

/// Show the quick-ask panel and make it the key window so the input
/// receives keystrokes immediately. With NonactivatingPanel set this
/// does NOT bring Corivo to the foreground; focus returns to the
/// previously-active app naturally on `hide_quick_ask`.
///
/// On non-macOS (which doesn't ship today, but the code is generic)
/// this falls back to the regular `WebviewWindow::show` + `set_focus`.
///
/// MUST run on the macOS main thread.
pub fn show_quick_ask<R: Runtime>(window: &WebviewWindow<R>) {
    #[cfg(target_os = "macos")]
    {
        let app = window.app_handle();
        let label = window.label();
        match app.get_webview_panel(label) {
            Ok(panel) => {
                panel.show_and_make_key();
                return;
            }
            Err(error) => {
                tracing::warn!(?error, "quick_ask.show.panel_missing_falling_back");
            }
        }
    }
    let _ = window.show();
    let _ = window.set_focus();
}

/// Hide the quick-ask panel. On macOS this `orderOut:`s the NSPanel,
/// which lets the previously-active app reclaim key-window status.
///
/// MUST run on the macOS main thread.
pub fn hide_quick_ask<R: Runtime>(window: &WebviewWindow<R>) {
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

/// Cheap visibility probe used by the toggle path in
/// `on_quick_ask_hotkey`. After the conversion the underlying
/// `NSWindow::isVisible` still works through the WebviewWindow handle
/// (NSPanel is an NSWindow subclass), but going through the panel
/// handle keeps the layering uniform with show/hide above.
pub fn is_quick_ask_visible<R: Runtime>(window: &WebviewWindow<R>) -> bool {
    #[cfg(target_os = "macos")]
    {
        let app = window.app_handle();
        let label = window.label();
        if let Ok(panel) = app.get_webview_panel(label) {
            return panel.is_visible();
        }
    }
    window.is_visible().unwrap_or(false)
}
