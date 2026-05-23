//! `NSException`-safe wrapper for synchronous Objective-C FFI calls.
//!
//! Several capture-pipeline paths reach into ObjC on macOS:
//! - `xcap` → CoreGraphics `CGWindowListCreateImage` (deprecated on 14+,
//!   removed-in-spirit on Tahoe / 26)
//! - `accessibility-sys` → `AXUIElementCopyAttributeValue` etc.
//! - `objc2-vision` → `VNRecognizeTextRequest`
//!
//! Any of these can raise an `NSException` (e.g. when the OS no longer
//! supports the API on a new macOS version). NSException unwinds
//! through the Rust FFI boundary, gets caught by `std::panic::catch_unwind`
//! as a *foreign* exception, and the Rust runtime then **aborts the
//! process** with `Rust cannot catch foreign exceptions`. There is no
//! way to recover from that point.
//!
//! `objc2::exception::catch` wraps the closure with `@try { ... }
//! @catch (NSException *) { ... }` on the ObjC side, converting the
//! exception into a `Result` *before* it unwinds into Rust. Use this
//! around every `spawn_blocking` body that calls into AppKit /
//! CoreGraphics / Vision / AX / etc.
//!
//! On non-macOS builds this is a no-op pass-through.

#[cfg(target_os = "macos")]
use std::panic::AssertUnwindSafe;

use super::error::{ExtractorError, Result};

/// Run `closure` with an `NSException` guard. On macOS, an exception
/// becomes `ExtractorError::Failed("nsexception: <name>: <reason>")`.
/// On other platforms this is exactly `closure()`.
///
/// `closure` is `AssertUnwindSafe`-wrapped because most call sites
/// hold non-`UnwindSafe` data (e.g. `&mut Vec<u8>` for image encoders);
/// catching an exception means we don't continue with the corrupted
/// state — the call site already returns the `Err` immediately.
pub fn catch_nsexception<T, F>(closure: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    catch_nsexception_with(closure, |summary| {
        ExtractorError::Failed(format!("nsexception: {summary}"))
    })
}

/// Same as [`catch_nsexception`] but the caller chooses how to
/// translate the exception summary into their own error type. Used by
/// `services::capture_pipeline::screen_capture` which returns
/// `crate::error::CorivoError`, not `ExtractorError`.
pub fn catch_nsexception_with<T, E, F, G>(closure: F, map_err: G) -> std::result::Result<T, E>
where
    F: FnOnce() -> std::result::Result<T, E>,
    G: FnOnce(String) -> E,
{
    #[cfg(target_os = "macos")]
    {
        // `AssertUnwindSafe<F>` implements both `FnOnce()` and
        // `UnwindSafe` for any `F: FnOnce()`, so passing it directly
        // satisfies `objc2::exception::catch`'s bounds without
        // requiring callers to thread `UnwindSafe` through every
        // closure they hand us.
        match objc2::exception::catch(AssertUnwindSafe(closure)) {
            Ok(inner) => inner,
            Err(maybe_exc) => {
                let summary = describe_exception(maybe_exc.as_deref());
                tracing::error!(%summary, "extractor.nsexception_caught");
                Err(map_err(summary))
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = map_err;
        closure()
    }
}

#[cfg(target_os = "macos")]
fn describe_exception(exc: Option<&objc2::exception::Exception>) -> String {
    let Some(exc) = exc else {
        return "<no exception attached>".to_string();
    };
    // `Exception`'s Display impl invokes `[NSException name]` +
    // `[NSException reason]` when the underlying object is in fact an
    // NSException (the common case); otherwise it falls back to the
    // raw class + address. Either way it never throws.
    exc.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_returns_ok() {
        let result: Result<i32> = catch_nsexception(|| Ok(42));
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn passthrough_returns_inner_err() {
        let result: Result<i32> = catch_nsexception(|| Err(ExtractorError::Failed("inner".into())));
        match result {
            Err(ExtractorError::Failed(msg)) => assert_eq!(msg, "inner"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }
}
