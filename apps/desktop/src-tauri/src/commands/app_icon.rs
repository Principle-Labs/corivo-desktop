//! App-icon lookup for the Quick Ask FocusCard.
//!
//! Resolution chain:
//!   1. `NSRunningApplication.runningApplicationsWithBundleIdentifier:`
//!      → `.icon()`. The frontmost app *is* running, so its NSImage is
//!      already in LaunchServices' running-process table — no on-disk
//!      index walk required. This is the reliable path for apps
//!      installed outside /Applications or renamed since LS last
//!      scanned them (which is why bare
//!      `URLForApplicationWithBundleIdentifier:` was missing apps like
//!      "Luck" in the first cut).
//!   2. Fallback: `NSWorkspace.URLForApplicationWithBundleIdentifier:`
//!      → `iconForFile:`. Catches the rare case the app quit between
//!      probe and icon lookup.
//!
//! Encoding chain (NSImage → PNG):
//!   * `imageRepsWithData` on the TIFF representation gives us every
//!     pixel rep packed in the .icns. We pick the smallest one ≥ 64px
//!     to stay sharp on retina without bloating the data URL with the
//!     full 1024px rep.
//!   * `NSBitmapImageRep.representationUsingType:` produces PNG bytes
//!     natively — no `image` crate decode, which silently failed on
//!     some multi-rep TIFFs (the original symptom: many app icons not
//!     showing up).
//!
//! Cache stores positive hits only. A failed lookup re-queries on the
//! next call so a transient miss self-heals.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use tauri::AppHandle;

static ICON_CACHE: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Smallest rep we'll accept. 32px display × 2x retina = 64 logical;
/// anything smaller pixelates on a retina display. Picking the
/// smallest rep ≥ this keeps the data URL compact.
#[cfg(target_os = "macos")]
const TARGET_PIXEL_SIZE: f64 = 64.0;

#[tauri::command]
pub async fn get_app_icon(bundle_id: String, app: AppHandle) -> Result<Option<String>, String> {
    if let Some(hit) = ICON_CACHE.lock().unwrap().get(&bundle_id).cloned() {
        return Ok(Some(hit));
    }

    // NSWorkspace + NSImage ops must run on the macOS main thread —
    // same constraint quick_ask_set_height and the foreground probe
    // honor. Dispatching from a tokio worker leaks NSException through
    // FFI and aborts the runtime.
    let (tx, rx) = tokio::sync::oneshot::channel();
    let bundle_for_main = bundle_id.clone();
    app.run_on_main_thread(move || {
        let _ = tx.send(fetch_icon_data_url(&bundle_for_main));
    })
    .map_err(|e| e.to_string())?;

    let result = rx.await.map_err(|e| e.to_string())?;
    if let Some(url) = &result {
        ICON_CACHE.lock().unwrap().insert(bundle_id, url.clone());
    } else {
        tracing::warn!(
            bundle_id = %bundle_id,
            "app_icon.fetch.miss"
        );
    }
    Ok(result)
}

#[cfg(target_os = "macos")]
fn fetch_icon_data_url(bundle_id: &str) -> Option<String> {
    use objc2_foundation::NSString;

    let bundle_ns = NSString::from_str(bundle_id);
    let (icon, source) = match icon_from_running_app(&bundle_ns) {
        Some(icon) => (icon, "running_app"),
        None => match icon_from_launch_services(&bundle_ns) {
            Some(icon) => (icon, "launch_services"),
            None => {
                tracing::warn!(
                    bundle_id = %bundle_id,
                    "app_icon.resolve.no_icon"
                );
                return None;
            }
        },
    };

    match encode_icon_as_data_url(&icon) {
        Some(url) => {
            tracing::debug!(
                bundle_id = %bundle_id,
                source = source,
                bytes = url.len(),
                "app_icon.resolve.ok"
            );
            Some(url)
        }
        None => {
            tracing::warn!(
                bundle_id = %bundle_id,
                source = source,
                "app_icon.encode.failed"
            );
            None
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn fetch_icon_data_url(_bundle_id: &str) -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
fn icon_from_running_app(
    bundle_ns: &objc2_foundation::NSString,
) -> Option<objc2::rc::Retained<objc2_app_kit::NSImage>> {
    use objc2_app_kit::NSRunningApplication;

    let apps = NSRunningApplication::runningApplicationsWithBundleIdentifier(bundle_ns);
    apps.iter().find_map(|app| app.icon())
}

#[cfg(target_os = "macos")]
fn icon_from_launch_services(
    bundle_ns: &objc2_foundation::NSString,
) -> Option<objc2::rc::Retained<objc2_app_kit::NSImage>> {
    use objc2_app_kit::NSWorkspace;

    let workspace = NSWorkspace::sharedWorkspace();
    let app_url = workspace.URLForApplicationWithBundleIdentifier(bundle_ns)?;
    let path_ns = app_url.path()?;
    Some(workspace.iconForFile(&path_ns))
}

/// NSImage → NSBitmapImageRep (closest to TARGET_PIXEL_SIZE) → PNG
/// bytes → base64 data URL. Goes through AppKit's own PNG encoder so
/// we don't depend on the `image` crate's TIFF decoder behavior on
/// macOS's multi-rep .icns dumps.
#[cfg(target_os = "macos")]
fn encode_icon_as_data_url(icon: &objc2_app_kit::NSImage) -> Option<String> {
    use base64::Engine;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
    use objc2_foundation::NSDictionary;

    let tiff = icon.TIFFRepresentation()?;
    let reps = NSBitmapImageRep::imageRepsWithData(&tiff);
    if reps.len() == 0 {
        return None;
    }

    // Pick the smallest rep with both dimensions ≥ TARGET. Fall back
    // to the largest rep if every rep is smaller than target (tiny
    // icons exist — old apps with only 16×16 reps).
    let mut best: Option<objc2::rc::Retained<NSBitmapImageRep>> = None;
    let mut best_dim = f64::INFINITY;
    let mut largest: Option<objc2::rc::Retained<NSBitmapImageRep>> = None;
    let mut largest_dim = 0.0_f64;
    for rep in reps.iter() {
        let Ok(bitmap) = rep.downcast::<NSBitmapImageRep>() else {
            continue;
        };
        let size = bitmap.size();
        let dim = size.width.min(size.height);
        if dim >= largest_dim {
            largest_dim = dim;
            largest = Some(bitmap.clone());
        }
        if dim >= TARGET_PIXEL_SIZE && dim < best_dim {
            best_dim = dim;
            best = Some(bitmap);
        }
    }
    let chosen = best.or(largest)?;

    // SAFETY: `properties` is the AppKit-conventional empty options
    // dict — every key it would carry (compression factor, gamma) is
    // optional, and no key requires a specific value type when absent.
    let empty_props = NSDictionary::new();
    let png_data = unsafe {
        chosen.representationUsingType_properties(NSBitmapImageFileType::PNG, &empty_props)
    }?;
    let bytes = png_data.to_vec();
    if bytes.is_empty() {
        return None;
    }

    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Some(format!("data:image/png;base64,{b64}"))
}
