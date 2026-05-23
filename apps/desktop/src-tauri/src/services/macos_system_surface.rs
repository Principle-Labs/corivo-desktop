use tauri::{AppHandle, Manager, Runtime};

#[cfg(target_os = "macos")]
use crate::error::CorivoError;
use crate::error::Result;

pub const DEFAULT_MAIN_WINDOW_LABEL: &str = "main";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacOSSystemSurfaceMode {
    RegularForeground,
    AccessoryBackground,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacOSSystemSurfaceInputs {
    pub main_window_visible: bool,
    pub has_other_foreground_surfaces: bool,
    pub overlay_visible: bool,
    pub is_recovering: bool,
    pub launch_hidden: bool,
}

pub fn resolve_macos_system_surface_mode(
    inputs: MacOSSystemSurfaceInputs,
) -> MacOSSystemSurfaceMode {
    let has_foreground_surface = inputs.main_window_visible || inputs.has_other_foreground_surfaces;
    if has_foreground_surface || (inputs.is_recovering && !inputs.launch_hidden) {
        MacOSSystemSurfaceMode::RegularForeground
    } else {
        // Overlay is intentionally a background-only surface and must not
        // promote the app back to a regular foreground app by itself.
        let _ = inputs.overlay_visible;
        MacOSSystemSurfaceMode::AccessoryBackground
    }
}

pub fn main_window_visible<R: Runtime>(app_handle: &AppHandle<R>, window_label: &str) -> bool {
    app_handle
        .get_webview_window(window_label)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false)
}

pub fn sync_macos_system_surface_from_runtime<R: Runtime>(
    app_handle: &AppHandle<R>,
    window_label: &str,
    overlay_visible: bool,
    is_recovering: bool,
    launch_hidden: bool,
) -> Result<MacOSSystemSurfaceMode> {
    let inputs = MacOSSystemSurfaceInputs {
        main_window_visible: main_window_visible(app_handle, window_label),
        has_other_foreground_surfaces: false,
        overlay_visible,
        is_recovering,
        launch_hidden,
    };
    let mode = resolve_macos_system_surface_mode(inputs);
    apply_macos_system_surface_mode(app_handle, mode)?;
    Ok(mode)
}

#[cfg(target_os = "macos")]
pub fn apply_macos_system_surface_mode<R: Runtime>(
    app_handle: &AppHandle<R>,
    mode: MacOSSystemSurfaceMode,
) -> Result<()> {
    let activation_policy = match mode {
        MacOSSystemSurfaceMode::RegularForeground => tauri::ActivationPolicy::Regular,
        MacOSSystemSurfaceMode::AccessoryBackground => tauri::ActivationPolicy::Accessory,
    };
    let dock_visible = matches!(mode, MacOSSystemSurfaceMode::RegularForeground);

    app_handle
        .set_activation_policy(activation_policy)
        .map_err(|error| {
            CorivoError::Internal(format!("设置 macOS activation policy 失败: {error}"))
        })?;
    app_handle
        .set_dock_visibility(dock_visible)
        .map_err(|error| CorivoError::Internal(format!("设置 Dock 可见性失败: {error}")))?;

    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn apply_macos_system_surface_mode<R: Runtime>(
    _app_handle: &AppHandle<R>,
    _mode: MacOSSystemSurfaceMode,
) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_visible_with_main_window_visible_uses_regular_foreground_mode() {
        let mode = resolve_macos_system_surface_mode(MacOSSystemSurfaceInputs {
            main_window_visible: true,
            has_other_foreground_surfaces: false,
            overlay_visible: false,
            is_recovering: false,
            launch_hidden: false,
        });

        assert_eq!(mode, MacOSSystemSurfaceMode::RegularForeground);
    }

    #[test]
    fn running_hidden_without_foreground_surfaces_uses_accessory_background_mode() {
        let mode = resolve_macos_system_surface_mode(MacOSSystemSurfaceInputs {
            main_window_visible: false,
            has_other_foreground_surfaces: false,
            overlay_visible: false,
            is_recovering: false,
            launch_hidden: false,
        });

        assert_eq!(mode, MacOSSystemSurfaceMode::AccessoryBackground);
    }

    #[test]
    fn overlay_visible_without_foreground_surfaces_stays_accessory_background_mode() {
        let mode = resolve_macos_system_surface_mode(MacOSSystemSurfaceInputs {
            main_window_visible: false,
            has_other_foreground_surfaces: false,
            overlay_visible: true,
            is_recovering: false,
            launch_hidden: false,
        });

        assert_eq!(mode, MacOSSystemSurfaceMode::AccessoryBackground);
    }

    #[test]
    fn hidden_recovery_launch_prefers_accessory_background_mode() {
        let mode = resolve_macos_system_surface_mode(MacOSSystemSurfaceInputs {
            main_window_visible: false,
            has_other_foreground_surfaces: false,
            overlay_visible: false,
            is_recovering: true,
            launch_hidden: true,
        });

        assert_eq!(mode, MacOSSystemSurfaceMode::AccessoryBackground);
    }

    #[test]
    fn visible_recovery_flow_prefers_regular_foreground_mode() {
        let mode = resolve_macos_system_surface_mode(MacOSSystemSurfaceInputs {
            main_window_visible: false,
            has_other_foreground_surfaces: false,
            overlay_visible: false,
            is_recovering: true,
            launch_hidden: false,
        });

        assert_eq!(mode, MacOSSystemSurfaceMode::RegularForeground);
    }
}
