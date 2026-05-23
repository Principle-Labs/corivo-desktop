//! VS Code adapter (spec §六 Phase 5).
//!
//! Window title format depends on the user's VSCode settings, but the
//! default is one of:
//!   - `"<file>"` (workspace-less single file)
//!   - `"<file> — <folder>"` (single folder)
//!   - `"<file> — <folder> — <profile>"` or
//!     `"<file> — <folder> [WSL: …]"`
//!
//! macOS uses the em-dash (`—`) by default; users can change it. We
//! handle em-dash + ASCII " - " separators.
//!
//! VS Code's own AX tree is famously thin (Monaco editor renders to
//! canvas); the dispatcher's OCR fallback is what supplies the body
//! text. The adapter's job is just to surface the structured payload.

use async_trait::async_trait;
use serde_json::json;

use super::super::error::Result;
use super::{AdapterContext, AdapterOutcome, FocusAdapter};

pub struct VsCodeAdapter;

impl VsCodeAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for VsCodeAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FocusAdapter for VsCodeAdapter {
    fn name(&self) -> &'static str {
        "vscode"
    }

    fn matches(&self, bundle_id: &str) -> bool {
        matches!(
            bundle_id,
            "com.microsoft.VSCode"
                | "com.microsoft.VSCodeInsiders"
                // Cursor + VSCodium share the same window-title shape.
                | "com.todesktop.230313mzl4w4u92"     // Cursor
                | "com.visualstudio.code.oss" // VSCodium
        )
    }

    async fn extract(&self, ctx: &AdapterContext<'_>) -> Result<AdapterOutcome> {
        let raw = ctx.window_title.trim();
        let parsed = parse_vscode_title(raw);
        Ok(AdapterOutcome::NeedsAxBody {
            payload: json!({
                "file": parsed.file,
                "folder": parsed.folder,
                "raw_window_title": raw,
            }),
        })
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct VsCodeTitle {
    file: Option<String>,
    folder: Option<String>,
}

fn parse_vscode_title(raw: &str) -> VsCodeTitle {
    if raw.is_empty() {
        return VsCodeTitle::default();
    }

    // Try em-dash first, then ASCII " - ". macOS VS Code defaults to
    // em-dash; explicit configs can override either way.
    let parts: Vec<&str> = if raw.contains(" — ") {
        raw.split(" — ").map(str::trim).collect()
    } else if raw.contains(" - ") {
        raw.split(" - ").map(str::trim).collect()
    } else {
        return VsCodeTitle {
            file: Some(raw.to_string()),
            folder: None,
        };
    };

    match parts.len() {
        1 => VsCodeTitle {
            file: Some(parts[0].to_string()),
            folder: None,
        },
        _ => VsCodeTitle {
            file: Some(parts[0].to_string()),
            folder: Some(parts[1].to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::snapshot_envelope::Trigger;

    fn ctx<'a>(t: &'a str) -> AdapterContext<'a> {
        AdapterContext {
            bundle_id: "com.microsoft.VSCode",
            app_name: "Code",
            window_title: t,
            pid: None,
            trigger: Trigger::FocusChange,
        }
    }

    #[test]
    fn matches_vscode_variants_and_friends() {
        let a = VsCodeAdapter::new();
        assert!(a.matches("com.microsoft.VSCode"));
        assert!(a.matches("com.microsoft.VSCodeInsiders"));
        assert!(a.matches("com.todesktop.230313mzl4w4u92"));
        assert!(!a.matches("com.apple.dt.Xcode"));
    }

    #[test]
    fn parses_file_and_folder_em_dash() {
        let parsed = parse_vscode_title("foo.rs — corivo-app");
        assert_eq!(
            parsed,
            VsCodeTitle {
                file: Some("foo.rs".into()),
                folder: Some("corivo-app".into()),
            }
        );
    }

    #[test]
    fn parses_file_and_folder_ascii_dash() {
        let parsed = parse_vscode_title("README.md - my-project - Workspace");
        assert_eq!(parsed.file.as_deref(), Some("README.md"));
        assert_eq!(parsed.folder.as_deref(), Some("my-project"));
    }

    #[test]
    fn keeps_raw_when_no_separator() {
        let parsed = parse_vscode_title("welcome.md");
        assert_eq!(parsed.file.as_deref(), Some("welcome.md"));
        assert!(parsed.folder.is_none());
    }

    #[test]
    fn empty_is_empty() {
        assert_eq!(parse_vscode_title(""), VsCodeTitle::default());
    }

    #[tokio::test]
    async fn extract_populates_payload() {
        let a = VsCodeAdapter::new();
        let out = a.extract(&ctx("lib.rs — corivo-app")).await.expect("ok");
        match out {
            AdapterOutcome::NeedsAxBody { payload } => {
                assert_eq!(payload["file"], "lib.rs");
                assert_eq!(payload["folder"], "corivo-app");
            }
            _ => panic!("expected NeedsAxBody, got {out:?}"),
        }
    }
}
