//! App-icon lookup for the Quick Ask FocusCard.
//!
//! macOS resolution chain:
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
//! Windows resolution chain:
//!   The capture helper sends `bundle_id` as the exe basename only
//!   (`WXWork.exe`) — see `foreground_monitor.cpp::exe_basename_for`.
//!   We don't get the full path, so we recover it by walking running
//!   processes via `EnumProcesses` and matching basenames. Any failure
//!   along the way short-circuits to `None` and the FocusCard falls
//!   back to its placeholder — installed UWP/MSIX apps, processes that
//!   already exited, and exes whose icon resources are non-standard
//!   all degrade gracefully rather than blocking the UI.
//!     1. `EnumProcesses` + `QueryFullProcessImageNameW` → full exe path.
//!     2. `PrivateExtractIconsW(path, 0, 256, 256, …)` → largest HICON
//!        available on disk. Shell scales down if the exe doesn't have
//!        a 256px rep.
//!     3. `GetIconInfo` + `GetDIBits` against a top-down 32-bit BGRA
//!        DIB → swap to RGBA → `image::codecs::png::PngEncoder`.
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

#[cfg(target_os = "windows")]
fn fetch_icon_data_url(bundle_id: &str) -> Option<String> {
    use base64::Engine;

    let exe_path = match find_process_exe_path(bundle_id) {
        Some(p) => p,
        None => {
            tracing::warn!(
                bundle_id = %bundle_id,
                "app_icon.resolve.no_process"
            );
            return None;
        }
    };

    let png = match extract_icon_png(&exe_path) {
        Some(bytes) => bytes,
        None => {
            tracing::warn!(
                bundle_id = %bundle_id,
                exe_path = %exe_path.display(),
                "app_icon.encode.failed"
            );
            return None;
        }
    };

    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    tracing::debug!(
        bundle_id = %bundle_id,
        bytes = b64.len(),
        "app_icon.resolve.ok"
    );
    Some(format!("data:image/png;base64,{b64}"))
}

/// Walks the running-process table to reverse-map an exe basename
/// (`WXWork.exe`) to a full path (`C:\Program Files\…\WXWork.exe`).
/// The helper only sends the basename over IPC, so we rediscover the
/// path here. Cheap: 4096-PID buffer + one `OpenProcess` per PID, and
/// the FocusCard caches positive hits.
#[cfg(target_os = "windows")]
fn find_process_exe_path(exe_basename: &str) -> Option<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::ProcessStatus::EnumProcesses;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let mut pids = vec![0u32; 4096];
    let mut bytes_returned: u32 = 0;
    let ok = unsafe {
        EnumProcesses(
            pids.as_mut_ptr(),
            (pids.len() * std::mem::size_of::<u32>()) as u32,
            &mut bytes_returned,
        )
    };
    if ok == 0 {
        return None;
    }
    let count =
        (bytes_returned as usize / std::mem::size_of::<u32>()).min(pids.len());

    for &pid in &pids[..count] {
        if pid == 0 {
            continue;
        }
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            continue;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let got = unsafe { QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len) };
        unsafe {
            CloseHandle(handle);
        }
        if got == 0 || len == 0 {
            continue;
        }
        let path = std::path::PathBuf::from(std::ffi::OsString::from_wide(&buf[..len as usize]));
        if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
            if name.eq_ignore_ascii_case(exe_basename) {
                return Some(path);
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn extract_icon_png(exe_path: &std::path::Path) -> Option<Vec<u8>> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, PrivateExtractIconsW, HICON};

    let wide: Vec<u16> = exe_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut hicon: HICON = std::ptr::null_mut();
    let mut icon_id: u32 = 0;
    // 256x256 hints Shell to hand back the highest-res rep packed in
    // the exe's icon group; it scales down if nothing that large is
    // available. nIcons=1 keeps the call cheap.
    let extracted = unsafe {
        PrivateExtractIconsW(
            wide.as_ptr(),
            0,
            256,
            256,
            &mut hicon,
            &mut icon_id,
            1,
            0,
        )
    };
    if extracted == 0 || extracted == u32::MAX || hicon.is_null() {
        return None;
    }

    let png = hicon_to_png(hicon);
    unsafe {
        DestroyIcon(hicon);
    }
    png
}

/// HICON → BGRA DIB → swap to RGBA → PNG. The PNG encoder is the
/// `image` crate's (already in deps for OCR thumbnails), not Shell's,
/// because Shell's IPicture / OleSavePictureFile path adds COM init
/// overhead for no benefit at this scale.
#[cfg(target_os = "windows")]
fn hicon_to_png(hicon: windows_sys::Win32::UI::WindowsAndMessaging::HICON) -> Option<Vec<u8>> {
    use image::ImageEncoder;
    use windows_sys::Win32::Graphics::Gdi::{
        DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetIconInfo, ICONINFO};

    let mut info: ICONINFO = unsafe { std::mem::zeroed() };
    if unsafe { GetIconInfo(hicon, &mut info) } == 0 {
        return None;
    }

    // Monochrome icons (rare for app exes) skip the color bitmap. Bail
    // rather than try to decode the AND mask — the FocusCard fallback
    // is more useful than a 1-bit silhouette.
    if info.hbmColor.is_null() {
        unsafe {
            if !info.hbmMask.is_null() {
                DeleteObject(info.hbmMask as _);
            }
        }
        return None;
    }

    let mut bmp: BITMAP = unsafe { std::mem::zeroed() };
    let got = unsafe {
        GetObjectW(
            info.hbmColor as _,
            std::mem::size_of::<BITMAP>() as i32,
            &mut bmp as *mut _ as *mut _,
        )
    };
    if got == 0 || bmp.bmWidth <= 0 || bmp.bmHeight <= 0 {
        unsafe {
            DeleteObject(info.hbmColor as _);
            if !info.hbmMask.is_null() {
                DeleteObject(info.hbmMask as _);
            }
        }
        return None;
    }

    let width = bmp.bmWidth;
    let height = bmp.bmHeight;
    let mut pixels = vec![0u8; (width as usize) * (height as usize) * 4];

    let mut bmi: BITMAPINFO = unsafe { std::mem::zeroed() };
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = width;
    // Negative height → top-down rows so we don't have to flip after
    // GetDIBits.
    bmi.bmiHeader.biHeight = -height;
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB;

    let hdc = unsafe { GetDC(std::ptr::null_mut()) };
    let lines = unsafe {
        GetDIBits(
            hdc,
            info.hbmColor,
            0,
            height as u32,
            pixels.as_mut_ptr() as _,
            &mut bmi,
            DIB_RGB_COLORS,
        )
    };
    unsafe {
        ReleaseDC(std::ptr::null_mut(), hdc);
        DeleteObject(info.hbmColor as _);
        if !info.hbmMask.is_null() {
            DeleteObject(info.hbmMask as _);
        }
    }

    if lines == 0 {
        return None;
    }

    // BGRA → RGBA. GDI hands back BB GG RR AA in memory order; the
    // PNG encoder wants RR GG BB AA. Alpha stays where it is.
    for chunk in pixels.chunks_exact_mut(4) {
        chunk.swap(0, 2);
    }

    let mut out = Vec::with_capacity(8192);
    let encoder = image::codecs::png::PngEncoder::new(&mut out);
    encoder
        .write_image(
            &pixels,
            width as u32,
            height as u32,
            image::ExtendedColorType::Rgba8,
        )
        .ok()?;
    Some(out)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
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
