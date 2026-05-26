//! Frontmost-application probe.
//!
//! **Phase 1**: macOS via `NSWorkspace.frontmostApplication` for bundle id /
//! name / pid. `window_title` and `url` stay `None` until Phase 2 lights up
//! the AX path (those need accessibility access; the v3 spec defers them).
//!
//! Windows resolves the same shape through the capture helper's
//! `foreground.current` RPC so Quick Ask can hand a real pid to UIA.

#[derive(Debug, Clone, Default)]
pub struct ForegroundProbe {
    pub bundle_id: Option<String>,
    pub name: Option<String>,
    pub pid: Option<i32>,
    pub window_title: Option<String>,
    pub url: Option<String>,
}

#[cfg(target_os = "macos")]
pub fn probe() -> ForegroundProbe {
    use objc2_app_kit::NSWorkspace;

    let workspace = NSWorkspace::sharedWorkspace();
    let Some(app) = workspace.frontmostApplication() else {
        return ForegroundProbe::default();
    };

    let bundle_id = app.bundleIdentifier().map(|s| s.to_string());
    let name = app.localizedName().map(|s| s.to_string());
    let pid = app.processIdentifier() as i32;

    // Pull the frontmost window title via xcap (CGWindowList under the
    // hood). Required by the browser adapter to parse `page_title` out
    // of "<tab> - Google Chrome" / "<tab> — Mozilla Firefox" / etc.;
    // without it BrowserAdapter sees an empty string and emits a null
    // page_title, which is why the Quick Ask FocusCard wasn't showing
    // tab subtitles.
    //
    // macOS 14+ gates `kCGWindowName` on Screen Recording. We preflight
    // before calling xcap so users who haven't granted the permission
    // don't get a `log::warn!` from xcap on every tick — they just
    // lose the tab-title enrichment until they grant it.
    let window_title = if screen_recording_permission_granted() {
        window_title_for_pid(pid)
    } else {
        None
    };

    ForegroundProbe {
        bundle_id,
        name,
        pid: Some(pid),
        window_title,
        url: None,
    }
}

#[cfg(target_os = "macos")]
pub async fn probe_current() -> ForegroundProbe {
    probe()
}

#[cfg(target_os = "macos")]
fn window_title_for_pid(target_pid: i32) -> Option<String> {
    // xcap returns windows roughly in z-order on macOS (CGWindowList
    // with `kCGWindowListOptionOnScreenOnly`), so the first match for
    // the target pid is the frontmost on-screen window — which is
    // exactly the one the user is interacting with. Skip minimized
    // windows so an old VSCode in the Dock doesn't shadow the actual
    // foreground Chrome window when both share pid via fast-switch
    // (rare, but cheap to guard against).
    let windows = xcap::Window::all().ok()?;
    windows
        .into_iter()
        .filter(|w| w.pid().map(|p| p as i32 == target_pid).unwrap_or(false))
        .filter(|w| !w.is_minimized().unwrap_or(false))
        .find_map(|w| {
            let title = w.title().ok()?;
            let trimmed = title.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
}

/// Read-only Screen Recording permission check. Does not prompt the
/// user — that flow runs through onboarding via
/// `request_screen_recording_permission`.
#[cfg(target_os = "macos")]
fn screen_recording_permission_granted() -> bool {
    extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
    }
    unsafe { CGPreflightScreenCaptureAccess() }
}

#[cfg(not(target_os = "macos"))]
pub fn probe() -> ForegroundProbe {
    ForegroundProbe::default()
}

#[cfg(not(target_os = "macos"))]
pub async fn probe_current() -> ForegroundProbe {
    let Some(client) = crate::services::capture_client::global::try_get() else {
        return ForegroundProbe::default();
    };
    match client.current_foreground().await {
        Ok(app) => ForegroundProbe {
            bundle_id: non_empty(app.bundle_id),
            name: non_empty(app.app_name),
            pid: app.pid,
            window_title: non_empty(app.window_title),
            url: None,
        },
        Err(error) => {
            tracing::debug!(?error, "foreground.probe_current_failed");
            ForegroundProbe::default()
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn non_empty(value: Option<String>) -> Option<String> {
    value.and_then(|s| {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}
