//! JetBrains IDE adapter — IntelliJ, WebStorm, PyCharm, GoLand, etc.
//!
//! Window title is typically:
//!   - `"<file>" - <project>"`  (single-file edit)
//!   - `"<project> – <file>"`   (project view)
//!   - `"<project> [<branch>] – …"` (with VCS plugin)
//!
//! JetBrains uses an en-dash (`–`, U+2013) by default. We accept both
//! the en-dash and ASCII `-` as separators.

use async_trait::async_trait;
use serde_json::json;

use super::super::error::Result;
use super::{AdapterContext, AdapterOutcome, FocusAdapter};

const JETBRAINS_BUNDLE_PREFIX: &str = "com.jetbrains.";

pub struct JetBrainsAdapter;

impl JetBrainsAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for JetBrainsAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FocusAdapter for JetBrainsAdapter {
    fn name(&self) -> &'static str {
        "jetbrains"
    }

    fn matches(&self, bundle_id: &str) -> bool {
        // JetBrains ships every IDE under `com.jetbrains.<ide>` —
        // intellij, webstorm, pycharm, goland, rider, clion, rubymine,
        // datagrip, phpstorm, rustrover, …. Prefix-match avoids
        // hard-coding the exact list.
        bundle_id.starts_with(JETBRAINS_BUNDLE_PREFIX)
    }

    async fn extract(&self, ctx: &AdapterContext<'_>) -> Result<AdapterOutcome> {
        let raw = ctx.window_title.trim();
        let parsed = parse_jetbrains_title(raw);
        Ok(AdapterOutcome::NeedsAxBody {
            payload: json!({
                "project": parsed.project,
                "file": parsed.file,
                "branch": parsed.branch,
                "raw_window_title": raw,
            }),
        })
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct JetBrainsTitle {
    project: Option<String>,
    file: Option<String>,
    branch: Option<String>,
}

fn parse_jetbrains_title(raw: &str) -> JetBrainsTitle {
    if raw.is_empty() {
        return JetBrainsTitle::default();
    }

    // The first segment can carry `[<branch>]` annotations; strip them
    // out and remember the branch.
    let separators = [" – ", " - "];
    let segments: Vec<&str> = separators
        .iter()
        .find_map(|sep| {
            if raw.contains(sep) {
                Some(raw.split(*sep).map(str::trim).collect::<Vec<_>>())
            } else {
                None
            }
        })
        .unwrap_or_else(|| vec![raw]);

    let (project, branch) = match segments.first().copied() {
        Some(first) => extract_branch(first),
        None => (None, None),
    };
    let file = segments.get(1).map(|s| s.to_string());

    JetBrainsTitle {
        project,
        file,
        branch,
    }
}

fn extract_branch(segment: &str) -> (Option<String>, Option<String>) {
    if let Some(open) = segment.rfind('[') {
        if let Some(close_rel) = segment[open + 1..].find(']') {
            let close = open + 1 + close_rel;
            let branch = segment[open + 1..close].trim().to_string();
            let mut project = String::new();
            project.push_str(segment[..open].trim_end());
            let after = segment[close + 1..].trim();
            if !after.is_empty() {
                if !project.is_empty() {
                    project.push(' ');
                }
                project.push_str(after);
            }
            return (
                if project.is_empty() {
                    None
                } else {
                    Some(project)
                },
                if branch.is_empty() {
                    None
                } else {
                    Some(branch)
                },
            );
        }
    }
    let trimmed = segment.trim();
    (
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        },
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::snapshot_envelope::Trigger;

    fn ctx<'a>(t: &'a str) -> AdapterContext<'a> {
        AdapterContext {
            bundle_id: "com.jetbrains.WebStorm",
            app_name: "WebStorm",
            window_title: t,
            pid: None,
            trigger: Trigger::FocusChange,
        }
    }

    #[test]
    fn matches_all_jetbrains_ides_via_prefix() {
        let a = JetBrainsAdapter::new();
        assert!(a.matches("com.jetbrains.intellij"));
        assert!(a.matches("com.jetbrains.WebStorm"));
        assert!(a.matches("com.jetbrains.pycharm"));
        assert!(a.matches("com.jetbrains.goland"));
        assert!(a.matches("com.jetbrains.rust-rover"));
        assert!(!a.matches("com.microsoft.VSCode"));
    }

    #[test]
    fn parses_project_with_branch_and_file() {
        let parsed = parse_jetbrains_title("widget [main] – widget.tsx");
        assert_eq!(parsed.project.as_deref(), Some("widget"));
        assert_eq!(parsed.branch.as_deref(), Some("main"));
        assert_eq!(parsed.file.as_deref(), Some("widget.tsx"));
    }

    #[test]
    fn parses_project_without_branch() {
        let parsed = parse_jetbrains_title("widget – widget.tsx");
        assert_eq!(parsed.project.as_deref(), Some("widget"));
        assert!(parsed.branch.is_none());
        assert_eq!(parsed.file.as_deref(), Some("widget.tsx"));
    }

    #[test]
    fn project_only_window() {
        let parsed = parse_jetbrains_title("widget");
        assert_eq!(parsed.project.as_deref(), Some("widget"));
        assert!(parsed.file.is_none());
    }

    #[test]
    fn empty_window_title() {
        assert_eq!(parse_jetbrains_title(""), JetBrainsTitle::default());
    }

    #[tokio::test]
    async fn extract_populates_payload_high_signal() {
        let a = JetBrainsAdapter::new();
        let out = a
            .extract(&ctx("monorepo [feature/x] – src/lib.rs"))
            .await
            .expect("ok");
        match out {
            AdapterOutcome::NeedsAxBody { payload } => {
                assert_eq!(payload["project"], "monorepo");
                assert_eq!(payload["branch"], "feature/x");
                assert_eq!(payload["file"], "src/lib.rs");
            }
            _ => panic!("expected NeedsAxBody, got {out:?}"),
        }
    }
}
