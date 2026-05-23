//! `dump_ax` — dev-only Accessibility tree dumper. macOS only.
//!
//! Pretty-prints the focused window's AX tree of the given pid as
//! JSON on stdout. Use it to scout per-app adapter selectors (Lark
//! message list role chain, sidebar nav structure, etc.) without
//! guessing.
//!
//! ```sh
//! # From the repo root:
//! cargo run --manifest-path apps/desktop/src-tauri/Cargo.toml \
//!     --bin dump_ax -- <pid> [--max-depth N] [--max-children N]
//!
//! # Common workflow — give yourself 5s to switch to Lark / Feishu:
//! sleep 5 && cargo run --manifest-path apps/desktop/src-tauri/Cargo.toml \
//!     --bin dump_ax -- $(pgrep -x Lark | head -1) > /tmp/lark-im.json
//! ```
//!
//! First run triggers an Accessibility permission prompt for the
//! built binary path (`target/debug/dump_ax`). Accept once via
//! System Settings → Privacy & Security → Accessibility, then re-run.

#[cfg(target_os = "macos")]
fn main() -> std::process::ExitCode {
    macos::run()
}

#[cfg(not(target_os = "macos"))]
fn main() -> std::process::ExitCode {
    eprintln!("dump_ax: macOS-only tool");
    std::process::ExitCode::from(1)
}

#[cfg(target_os = "macos")]
mod macos {
    use std::env;
    use std::ffi::c_void;
    use std::process::ExitCode;

    use accessibility_sys::{
        kAXErrorSuccess, kAXFocusedUIElementAttribute, kAXNumberOfCharactersAttribute,
        kAXParentAttribute, kAXSelectedTextAttribute, kAXSelectedTextRangeAttribute,
        kAXValueTypeCFRange, AXIsProcessTrusted, AXUIElementCopyAttributeValue,
        AXUIElementCopyParameterizedAttributeValue, AXUIElementCreateApplication,
        AXUIElementCreateSystemWide, AXUIElementRef, AXUIElementSetMessagingTimeout,
        AXValueGetType, AXValueGetValue, AXValueRef,
    };
    use core_foundation::array::{CFArray, CFArrayRef};
    use core_foundation::base::{CFRelease, CFType, CFTypeRef, TCFType};
    use core_foundation::boolean::{CFBoolean, CFBooleanRef};
    use core_foundation::number::{CFNumber, CFNumberRef};
    use core_foundation::string::{CFString, CFStringRef};
    use serde::Serialize;

    /// Per-RPC AX call timeout. Inherited by descendants once set on
    /// the application root. Matches `ax_extractor::AX_TIMEOUT`.
    const MESSAGING_TIMEOUT_SECS: f32 = 1.5;
    const DEFAULT_MAX_DEPTH: u32 = 30;
    /// Cap children per node so AXOutline / AXTable with thousands of
    /// rows don't blow up the JSON. Bump via `--max-children` if you
    /// need more.
    const DEFAULT_MAX_CHILDREN: usize = 200;

    #[derive(Serialize)]
    struct DumpOutput {
        pid: i32,
        trusted: bool,
        max_depth: u32,
        max_children: usize,
        focused_window: Option<AxNodeJson>,
        /// Selection probe — populated independently of the tree walk
        /// so we can scout `kAXSelectedText{,Range}` on the focused
        /// leaf even when the focused-window dump fails. See
        /// [`probe_selection`].
        #[serde(skip_serializing_if = "Option::is_none")]
        selection: Option<SelectionProbe>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    }

    #[derive(Serialize)]
    struct SelectionProbe {
        /// Which `AXFocusedUIElement` source resolved — `app` (the
        /// pid's own root) or `system_wide` (the global one). We try
        /// the app root first since it works whether or not the app
        /// is frontmost.
        source: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        focused: Option<FocusedElementInfo>,
        /// Cross-node selection via `AXSelectedTextMarkerRange` +
        /// `AXStringForTextMarkerRange`. This is the WebKit/Chromium
        /// path that survives selections spanning multiple AX nodes
        /// (where plain `AXSelectedText` returns null). Probe walks
        /// from the focused leaf up to its ancestors looking for an
        /// element that exposes the marker-range attribute.
        #[serde(skip_serializing_if = "Option::is_none")]
        marker: Option<MarkerProbe>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    }

    #[derive(Serialize)]
    struct MarkerProbe {
        /// Role of the AX element on which we found
        /// `AXSelectedTextMarkerRange`. May be the focused leaf
        /// itself or some ancestor (typically `AXWebArea` for
        /// browsers, `AXScrollArea` / a doc root for native apps).
        #[serde(skip_serializing_if = "Option::is_none")]
        found_on_role: Option<String>,
        /// Distance walked from the focused leaf — `0` means the
        /// leaf itself answered, `1` means its parent, etc.
        found_at_ancestor_depth: u32,
        /// Plain text of the cross-node selection, retrieved via
        /// the `AXStringForTextMarkerRange` parameterized attribute.
        /// Empty string means the marker range was present but
        /// covers zero characters (collapsed caret).
        #[serde(skip_serializing_if = "Option::is_none")]
        string: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        string_byte_len: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        string_utf16_len: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    }

    #[derive(Serialize)]
    struct FocusedElementInfo {
        #[serde(skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        subrole: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        identifier: Option<String>,
        /// Focused element's full text. Truncated for the JSON dump
        /// — the full string would balloon the output for big
        /// textareas / WebAreas. Lengths below are computed on the
        /// untruncated string.
        #[serde(skip_serializing_if = "Option::is_none")]
        value_preview: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        value_byte_len: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        value_utf16_len: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        value_char_len: Option<usize>,
        /// `AXNumberOfCharacters` — handy reference for what unit AX
        /// thinks "character" means in this context.
        #[serde(skip_serializing_if = "Option::is_none")]
        number_of_characters: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        selected_text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        selected_text_byte_len: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        selected_text_utf16_len: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        selected_text_char_len: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        selected_text_range: Option<RangeInfo>,
    }

    #[derive(Serialize)]
    struct RangeInfo {
        location: i64,
        length: i64,
    }

    /// Layout-compatible with `CFRange` from CoreFoundation
    /// (`{ CFIndex location; CFIndex length; }`). `CFIndex` is a
    /// signed long, i.e. `isize` on macOS. core-foundation 0.10
    /// doesn't re-export this type, so we shim it locally.
    #[repr(C)]
    struct CFRangeShim {
        location: isize,
        length: isize,
    }

    /// Cap so a fullscreen WebArea text dump doesn't bloat the JSON.
    const VALUE_PREVIEW_BYTES: usize = 4096;

    #[derive(Serialize)]
    struct AxNodeJson {
        #[serde(skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        subrole: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        help: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        identifier: Option<String>,
        children_total: usize,
        children_dumped: usize,
        children: Vec<AxNodeJson>,
    }

    pub fn run() -> ExitCode {
        let args: Vec<String> = env::args().collect();
        let pid = match args.get(1).and_then(|s| s.parse::<i32>().ok()) {
            Some(p) => p,
            None => {
                eprintln!(
                    "usage: dump_ax <pid> [--max-depth N] [--max-children N]\n\
                     hint: get pid via `pgrep -x Lark` or `pgrep -x 'Feishu'`"
                );
                return ExitCode::from(2);
            }
        };
        let max_depth = parse_named_flag(&args, "--max-depth", DEFAULT_MAX_DEPTH);
        let max_children = parse_named_flag(&args, "--max-children", DEFAULT_MAX_CHILDREN);

        let output = dump_for_pid(pid, max_depth, max_children);

        match serde_json::to_string_pretty(&output) {
            Ok(s) => {
                println!("{s}");
                if output.error.is_some() {
                    ExitCode::from(1)
                } else {
                    ExitCode::SUCCESS
                }
            }
            Err(e) => {
                eprintln!("serialize error: {e}");
                ExitCode::from(3)
            }
        }
    }

    fn parse_named_flag<T: std::str::FromStr>(args: &[String], flag: &str, default: T) -> T {
        args.windows(2)
            .find(|w| w[0] == flag)
            .and_then(|w| w[1].parse().ok())
            .unwrap_or(default)
    }

    fn dump_for_pid(pid: i32, max_depth: u32, max_children: usize) -> DumpOutput {
        let trusted = unsafe { AXIsProcessTrusted() };
        if !trusted {
            return DumpOutput {
                pid,
                trusted,
                max_depth,
                max_children,
                focused_window: None,
                selection: None,
                error: Some(
                    "Accessibility permission missing for this binary. Grant it via \
                     System Settings → Privacy & Security → Accessibility, then re-run."
                        .into(),
                ),
            };
        }

        let app = unsafe { AXUIElementCreateApplication(pid) };
        if app.is_null() {
            return DumpOutput {
                pid,
                trusted,
                max_depth,
                max_children,
                focused_window: None,
                selection: None,
                error: Some(format!("AXUIElementCreateApplication({pid}) returned null")),
            };
        }
        let app = AxOwned(app);
        unsafe { AXUIElementSetMessagingTimeout(app.0, MESSAGING_TIMEOUT_SECS) };

        let selection = Some(probe_selection(app.0));

        let focused = match copy_attribute(app.0, "AXFocusedWindow") {
            Some(v) => v,
            None => {
                return DumpOutput {
                    pid,
                    trusted,
                    max_depth,
                    max_children,
                    focused_window: None,
                    selection,
                    error: Some(
                        "focused window unavailable — is the app frontmost and a window open?"
                            .into(),
                    ),
                };
            }
        };
        let window = focused.as_concrete_TypeRef() as AXUIElementRef;
        let node = walk(window, 0, max_depth, max_children);
        drop(focused);

        DumpOutput {
            pid,
            trusted,
            max_depth,
            max_children,
            focused_window: Some(node),
            selection,
            error: None,
        }
    }

    /// Read the focused leaf via `AXFocusedUIElement` (app root first,
    /// then system-wide as fallback) and surface the attributes we
    /// care about for a "highlight selection in extracted text"
    /// pipeline.
    ///
    /// Three open questions this probe is meant to answer:
    /// 1. Does the focused element actually expose `AXSelectedText`
    ///    in Chrome / Safari? (Or only in native textareas?)
    /// 2. When the user selects across DOM elements, does
    ///    `AXSelectedText` contain the full string, only the leaf's
    ///    portion, or empty?
    /// 3. Are the offsets in `AXSelectedTextRange` UTF-16 code units
    ///    or UTF-8 bytes? Compare `range.{location,length}` against
    ///    `value_byte_len` / `value_utf16_len` / `value_char_len`.
    fn probe_selection(app: AXUIElementRef) -> SelectionProbe {
        let (source, focused_cf) = match copy_attribute(app, kAXFocusedUIElementAttribute) {
            Some(v) => ("app", v),
            None => {
                let system = unsafe { AXUIElementCreateSystemWide() };
                if system.is_null() {
                    return SelectionProbe {
                        source: "none",
                        focused: None,
                        marker: None,
                        error: Some(
                            "AXFocusedUIElement unavailable on app root, and \
                             AXUIElementCreateSystemWide returned null."
                                .into(),
                        ),
                    };
                }
                let system = AxOwned(system);
                unsafe { AXUIElementSetMessagingTimeout(system.0, MESSAGING_TIMEOUT_SECS) };
                match copy_attribute(system.0, kAXFocusedUIElementAttribute) {
                    Some(v) => ("system_wide", v),
                    None => {
                        return SelectionProbe {
                            source: "none",
                            focused: None,
                            marker: None,
                            error: Some(
                                "AXFocusedUIElement unavailable on both app root \
                                 and system-wide element."
                                    .into(),
                            ),
                        };
                    }
                }
            }
        };

        let element = focused_cf.as_concrete_TypeRef() as AXUIElementRef;

        let marker = Some(probe_text_marker(element));

        let role = copy_text(element, "AXRole");
        let subrole = copy_text(element, "AXSubrole");
        let title = copy_text(element, "AXTitle");
        let identifier = copy_text(element, "AXIdentifier");

        let value = copy_text(element, "AXValue");
        let (value_preview, value_byte_len, value_utf16_len, value_char_len) = match &value {
            Some(s) => (
                Some(truncate_for_preview(s, VALUE_PREVIEW_BYTES)),
                Some(s.len()),
                Some(s.encode_utf16().count()),
                Some(s.chars().count()),
            ),
            None => (None, None, None, None),
        };

        let number_of_characters = copy_attribute(element, kAXNumberOfCharactersAttribute)
            .as_ref()
            .and_then(|v| cf_value_as_text(v).and_then(|s| s.parse::<i64>().ok()));

        let selected_text = copy_text(element, kAXSelectedTextAttribute);
        let (selected_text_byte_len, selected_text_utf16_len, selected_text_char_len) =
            match &selected_text {
                Some(s) => (
                    Some(s.len()),
                    Some(s.encode_utf16().count()),
                    Some(s.chars().count()),
                ),
                None => (None, None, None),
            };

        let selected_text_range = copy_cf_range(element, kAXSelectedTextRangeAttribute);

        drop(focused_cf);

        SelectionProbe {
            source,
            marker,
            focused: Some(FocusedElementInfo {
                role,
                subrole,
                title,
                identifier,
                value_preview,
                value_byte_len,
                value_utf16_len,
                value_char_len,
                number_of_characters,
                selected_text,
                selected_text_byte_len,
                selected_text_utf16_len,
                selected_text_char_len,
                selected_text_range,
            }),
            error: None,
        }
    }

    /// Walk from the focused leaf toward the root, looking for an
    /// element that exposes `AXSelectedTextMarkerRange`. Once found,
    /// retrieve the user's selection as plain text via the
    /// `AXStringForTextMarkerRange` parameterized attribute on the
    /// same element.
    ///
    /// We never crack open the AXTextMarkerRange — it's an opaque
    /// CF type and we just round-trip it as the parameter to the
    /// parameterized-attribute call.
    fn probe_text_marker(focused: AXUIElementRef) -> MarkerProbe {
        const MAX_WALK: u32 = 16;

        // Hold every ancestor `CFType` alive for the duration of the
        // walk: `as_concrete_TypeRef()` is only valid while the
        // owning CFType is still in scope.
        let mut ancestors: Vec<CFType> = Vec::new();
        let mut current = focused;
        let mut depth: u32 = 0;

        loop {
            if let Some(range_cf) = copy_attribute(current, "AXSelectedTextMarkerRange") {
                let role = copy_text(current, "AXRole");
                let string = copy_param_string(current, "AXStringForTextMarkerRange", &range_cf);
                let (byte_len, utf16_len, err) = match &string {
                    Some(s) => (Some(s.len()), Some(s.encode_utf16().count()), None),
                    None => (
                        None,
                        None,
                        Some(
                            "AXSelectedTextMarkerRange present but \
                             AXStringForTextMarkerRange returned null"
                                .into(),
                        ),
                    ),
                };
                drop(range_cf);
                drop(ancestors);
                return MarkerProbe {
                    found_on_role: role,
                    found_at_ancestor_depth: depth,
                    string,
                    string_byte_len: byte_len,
                    string_utf16_len: utf16_len,
                    error: err,
                };
            }

            if depth >= MAX_WALK {
                return MarkerProbe {
                    found_on_role: None,
                    found_at_ancestor_depth: depth,
                    string: None,
                    string_byte_len: None,
                    string_utf16_len: None,
                    error: Some(format!(
                        "walked {MAX_WALK} ancestors without finding AXSelectedTextMarkerRange"
                    )),
                };
            }

            let Some(parent_cf) = copy_attribute(current, kAXParentAttribute) else {
                return MarkerProbe {
                    found_on_role: None,
                    found_at_ancestor_depth: depth,
                    string: None,
                    string_byte_len: None,
                    string_utf16_len: None,
                    error: Some(format!(
                        "walked {depth} ancestor(s); AXParent chain ended before \
                         finding AXSelectedTextMarkerRange"
                    )),
                };
            };
            let parent = parent_cf.as_concrete_TypeRef() as AXUIElementRef;
            if parent.is_null() {
                return MarkerProbe {
                    found_on_role: None,
                    found_at_ancestor_depth: depth,
                    string: None,
                    string_byte_len: None,
                    string_utf16_len: None,
                    error: Some(format!(
                        "walked {depth} ancestor(s); AXParent returned a null element"
                    )),
                };
            }
            ancestors.push(parent_cf);
            current = parent;
            depth += 1;
        }
    }

    /// Call a parameterized AX attribute that takes a single CF
    /// parameter and returns a string. Returns `None` if the
    /// attribute is unsupported or the result isn't string-coercible.
    fn copy_param_string(
        element: AXUIElementRef,
        attribute: &str,
        parameter: &CFType,
    ) -> Option<String> {
        let key = CFString::new(attribute);
        let mut value: CFTypeRef = std::ptr::null();
        let err = unsafe {
            AXUIElementCopyParameterizedAttributeValue(
                element,
                key.as_concrete_TypeRef(),
                parameter.as_CFTypeRef(),
                &mut value,
            )
        };
        if err != kAXErrorSuccess || value.is_null() {
            return None;
        }
        let cf = unsafe { CFType::wrap_under_create_rule(value) };
        cf_value_as_text(&cf)
    }

    /// Decode an `AXValue`-wrapped `CFRange` (i.e. `kAXValueTypeCFRange`)
    /// into a plain `{location, length}` pair. Returns `None` if the
    /// attribute is missing or its inner `AXValue` isn't a CFRange.
    fn copy_cf_range(element: AXUIElementRef, attribute: &str) -> Option<RangeInfo> {
        let v = copy_attribute(element, attribute)?;
        let ax_value = v.as_concrete_TypeRef() as AXValueRef;
        if ax_value.is_null() {
            return None;
        }
        let actual_type = unsafe { AXValueGetType(ax_value) };
        if actual_type != kAXValueTypeCFRange {
            return None;
        }
        let mut range = CFRangeShim {
            location: 0,
            length: 0,
        };
        let ok = unsafe {
            AXValueGetValue(
                ax_value,
                kAXValueTypeCFRange,
                &mut range as *mut CFRangeShim as *mut c_void,
            )
        };
        if !ok {
            return None;
        }
        Some(RangeInfo {
            location: range.location as i64,
            length: range.length as i64,
        })
    }

    /// Truncate at a UTF-8 boundary so we don't slice multi-byte
    /// sequences in half when previewing a large `AXValue`.
    fn truncate_for_preview(s: &str, max_bytes: usize) -> String {
        if s.len() <= max_bytes {
            return s.to_string();
        }
        let mut end = max_bytes;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…[truncated, {} bytes total]", &s[..end], s.len())
    }

    fn walk(
        element: AXUIElementRef,
        depth: u32,
        max_depth: u32,
        max_children: usize,
    ) -> AxNodeJson {
        let role = copy_text(element, "AXRole");
        let subrole = copy_text(element, "AXSubrole");
        let title = copy_text(element, "AXTitle");
        let value = copy_text(element, "AXValue");
        let description = copy_text(element, "AXDescription");
        let help = copy_text(element, "AXHelp");
        let identifier = copy_text(element, "AXIdentifier");

        let (children_total, children) = if depth >= max_depth {
            (count_children(element), Vec::new())
        } else {
            walk_children(element, depth, max_depth, max_children)
        };

        let dumped = children.len();
        AxNodeJson {
            role,
            subrole,
            title,
            value,
            description,
            help,
            identifier,
            children_total,
            children_dumped: dumped,
            children,
        }
    }

    fn walk_children(
        element: AXUIElementRef,
        depth: u32,
        max_depth: u32,
        max_children: usize,
    ) -> (usize, Vec<AxNodeJson>) {
        let Some(children_cf) = copy_attribute(element, "AXChildren") else {
            return (0, Vec::new());
        };
        let arr = unsafe {
            CFArray::<*const c_void>::wrap_under_get_rule(
                children_cf.as_concrete_TypeRef() as CFArrayRef
            )
        };
        let total = arr.len() as usize;
        let take = total.min(max_children);
        let mut out = Vec::with_capacity(take);
        for i in 0..take {
            if let Some(item) = arr.get(i as isize) {
                let child = *item as AXUIElementRef;
                if !child.is_null() {
                    out.push(walk(child, depth + 1, max_depth, max_children));
                }
            }
        }
        (total, out)
    }

    fn count_children(element: AXUIElementRef) -> usize {
        let Some(children_cf) = copy_attribute(element, "AXChildren") else {
            return 0;
        };
        let arr = unsafe {
            CFArray::<*const c_void>::wrap_under_get_rule(
                children_cf.as_concrete_TypeRef() as CFArrayRef
            )
        };
        arr.len() as usize
    }

    fn copy_attribute(element: AXUIElementRef, attribute: &str) -> Option<CFType> {
        let key = CFString::new(attribute);
        let mut value: CFTypeRef = std::ptr::null();
        let err = unsafe {
            AXUIElementCopyAttributeValue(element, key.as_concrete_TypeRef(), &mut value)
        };
        if err != kAXErrorSuccess || value.is_null() {
            return None;
        }
        Some(unsafe { CFType::wrap_under_create_rule(value) })
    }

    fn copy_text(element: AXUIElementRef, attribute: &str) -> Option<String> {
        let v = copy_attribute(element, attribute)?;
        cf_value_as_text(&v)
    }

    /// Coerce common CFTypes returned by AX into a string. Rectangles
    /// and point/size AXValues are skipped on purpose — they're noise
    /// for selector scouting and need extra unwrapping.
    fn cf_value_as_text(value: &CFType) -> Option<String> {
        let type_id = value.type_of();
        if type_id == CFString::type_id() {
            let raw = value.as_concrete_TypeRef() as CFStringRef;
            if raw.is_null() {
                return None;
            }
            return Some(unsafe { CFString::wrap_under_get_rule(raw) }.to_string());
        }
        if type_id == CFBoolean::type_id() {
            let raw = value.as_concrete_TypeRef() as CFBooleanRef;
            let b = unsafe { CFBoolean::wrap_under_get_rule(raw) };
            return Some(if b == CFBoolean::true_value() {
                "true".into()
            } else {
                "false".into()
            });
        }
        if type_id == CFNumber::type_id() {
            let raw = value.as_concrete_TypeRef() as CFNumberRef;
            let n = unsafe { CFNumber::wrap_under_get_rule(raw) };
            if let Some(v) = n.to_i64() {
                return Some(v.to_string());
            }
            if let Some(v) = n.to_f64() {
                return Some(v.to_string());
            }
            return None;
        }
        None
    }

    /// RAII for a `+1` retained AXUIElementRef.
    struct AxOwned(AXUIElementRef);
    impl Drop for AxOwned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0 as CFTypeRef) }
            }
        }
    }
}
