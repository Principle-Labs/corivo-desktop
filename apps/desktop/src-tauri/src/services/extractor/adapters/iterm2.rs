//! iTerm2 / Terminal.app adapter.
//!
//! iTerm2 puts `<title> — <profile>` (or `<cwd>` if title is empty)
//! into the window title; Terminal.app uses `<cwd> — <user>@<host>` by
//! default. We surface whatever's there as a `cwd_or_title` field —
//! the recall layer doesn't need to be smarter than "user is in some
//! shell at <X>".
//!
//! Many shells write the running command into the title via the
//! prompt (`PROMPT_COMMAND` for bash / preexec for zsh), in which
//! case the adapter payload effectively carries the last command.

use async_trait::async_trait;
use serde_json::json;

use super::super::error::Result;
use super::{AdapterContext, AdapterOutcome, FocusAdapter};

pub struct ITerm2Adapter;

impl ITerm2Adapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ITerm2Adapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FocusAdapter for ITerm2Adapter {
    fn name(&self) -> &'static str {
        "iterm2"
    }

    fn matches(&self, bundle_id: &str) -> bool {
        matches!(
            bundle_id,
            "com.googlecode.iterm2"
                | "com.apple.Terminal"
                // Hyper, Wezterm — text-only titles too.
                | "co.zeit.hyper"
                | "com.github.wez.wezterm"
        )
    }

    async fn extract(&self, ctx: &AdapterContext<'_>) -> Result<AdapterOutcome> {
        let raw = ctx.window_title.trim();
        let parsed = split_terminal_title(raw);
        Ok(AdapterOutcome::NeedsAxBody {
            payload: json!({
                "cwd_or_title": parsed.head,
                "tail": parsed.tail,
                "raw_window_title": raw,
            }),
        })
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct TerminalTitle {
    head: Option<String>,
    tail: Option<String>,
}

fn split_terminal_title(raw: &str) -> TerminalTitle {
    if raw.is_empty() {
        return TerminalTitle::default();
    }
    for sep in [" — ", " - ", " · "] {
        if let Some(idx) = raw.find(sep) {
            let head = raw[..idx].trim();
            let tail = raw[idx + sep.len()..].trim();
            return TerminalTitle {
                head: optional(head),
                tail: optional(tail),
            };
        }
    }
    TerminalTitle {
        head: Some(raw.to_string()),
        tail: None,
    }
}

fn optional(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::snapshot_envelope::Trigger;

    fn ctx<'a>(t: &'a str) -> AdapterContext<'a> {
        AdapterContext {
            bundle_id: "com.googlecode.iterm2",
            app_name: "iTerm",
            window_title: t,
            pid: None,
            trigger: Trigger::FocusChange,
        }
    }

    #[test]
    fn matches_iterm_terminal_and_friends() {
        let a = ITerm2Adapter::new();
        assert!(a.matches("com.googlecode.iterm2"));
        assert!(a.matches("com.apple.Terminal"));
        assert!(a.matches("co.zeit.hyper"));
        assert!(!a.matches("com.microsoft.VSCode"));
    }

    #[test]
    fn splits_em_dash_separator() {
        let parsed = split_terminal_title("/Users/airbo/dev — zsh");
        assert_eq!(parsed.head.as_deref(), Some("/Users/airbo/dev"));
        assert_eq!(parsed.tail.as_deref(), Some("zsh"));
    }

    #[test]
    fn no_separator_is_whole_title() {
        let parsed = split_terminal_title("running cargo test");
        assert_eq!(parsed.head.as_deref(), Some("running cargo test"));
        assert!(parsed.tail.is_none());
    }

    #[tokio::test]
    async fn extract_populates_payload() {
        let a = ITerm2Adapter::new();
        let out = a
            .extract(&ctx("~/Developer/corivo - zsh"))
            .await
            .expect("ok");
        match out {
            AdapterOutcome::NeedsAxBody { payload } => {
                assert_eq!(payload["cwd_or_title"], "~/Developer/corivo");
                assert_eq!(payload["tail"], "zsh");
            }
            _ => panic!("expected NeedsAxBody, got {out:?}"),
        }
    }
}
