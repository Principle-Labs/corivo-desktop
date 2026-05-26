//! App-identifier exclusion engine (spec §六).
//!
//! **Phase 1 scope**: app-level only. The dispatcher checks the foreground
//! app identifier against a default blocklist (Signal, password managers, banking)
//! plus any user-added entries from `Config`. Domain-level exclusion (URL
//! category lookups) is deferred to Phase 2+ per the spec.
//!
//! When a check fires `Block { reason }`, capture_pipeline still writes a
//! frame row, but with `extraction_strategy = 'skipped'`, no AX/OCR text,
//! and `exclusion_match = reason`. Users see "I was in this app at this
//! time, but the content isn't stored" on `/timeline` (spec §六).

use std::collections::HashSet;

/// Result of an exclusion check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExclusionVerdict {
    Allow,
    /// Foreground app hit an explicit blocklist entry (default-curated
    /// or user-added). The capture_pipeline still writes a `'skipped'`
    /// frame row tagged with `reason` so the timeline reads "I was in
    /// this app at this time, but the content isn't stored".
    Block {
        reason: String,
    },
    /// Foreground app is Corivo itself (matched by [`matches_self`]).
    /// No frame row is written and no screenshot is taken — capturing
    /// our own UI just produces blank/recursive frames that pollute the
    /// timeline. Caller should silently no-op.
    BlockSelf,
}

impl ExclusionVerdict {
    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::Block { .. } | Self::BlockSelf)
    }
}

/// Default app blocklist. App identifiers only — no fuzzy matching, no globs.
///
/// Curated (not exhaustive) — covers the categories the spec calls out:
/// secure messengers, password managers, banking. Users add more via
/// `Config.exclusion.extra_app_bundle_ids` (the Settings → 隐私 list).
///
/// macOS entries are bundle ids and match by exact equality.
pub const DEFAULT_BLOCKED_BUNDLES: &[&str] = &[
    // Secure messengers
    "org.whispersystems.signal-desktop",
    "ph.telegra.Telegraph",
    // Password managers
    "com.apple.keychainaccess",
    "com.1password.1password",
    "com.1password.1password7",
    "com.agilebits.onepassword7",
    "com.agilebits.onepassword4",
    "com.bitwarden.desktop",
    "com.dashlane.dashlane",
    // Banking / personal finance (examples; users will add their own)
    "com.intuit.quicken.mac",
];

/// Windows entries are executable basenames reported by the capture helper
/// and match case-insensitively by exact equality.
pub const DEFAULT_BLOCKED_EXECUTABLE_NAMES: &[&str] = &[
    // Secure messengers
    "Signal.exe",
    "Telegram.exe",
    // Password managers
    "1Password.exe",
    "Bitwarden.exe",
    "Dashlane.exe",
    "Enpass.exe",
    "KeePass.exe",
    "KeePassXC.exe",
    // Banking / personal finance (examples; users will add their own)
    "Quicken.exe",
];

/// Bundle-id prefixes identifying Corivo's own windows on macOS. Matched by
/// exact equality OR `<prefix>.` so the production app
/// (`ai.corivo.desktop`), the dev build (`ai.corivo.desktop.dev`), and
/// any future helper bundle id are all covered — but not unrelated ids
/// that happen to share a substring. Hard-coded; not user-configurable
/// and never removable from the UI.
pub const SELF_BUNDLE_PREFIXES: &[&str] = &["ai.corivo.desktop"];

/// Windows helper reports the foreground executable basename in the same
/// `bundle_id` slot. Match exact exe names case-insensitively.
pub const SELF_EXECUTABLE_NAMES: &[&str] = &["corivo-app.exe"];

/// Read-only view onto the curated default blocklist — used by the
/// settings command surface to render "default vs user-added" rows
/// distinctly in the UI.
pub fn default_blocked_bundles() -> &'static [&'static str] {
    DEFAULT_BLOCKED_BUNDLES
}

/// Read-only view onto Windows executable defaults.
pub fn default_blocked_executables() -> &'static [&'static str] {
    DEFAULT_BLOCKED_EXECUTABLE_NAMES
}

/// Read-only view onto the self-exclusion prefixes. Surfaced to the
/// Settings UI so users can see "Corivo 自身" is hard-blocked even
/// though the prefix never appears in [`DEFAULT_BLOCKED_BUNDLES`].
pub fn self_bundle_prefixes() -> &'static [&'static str] {
    SELF_BUNDLE_PREFIXES
}

/// Read-only view onto the exact executable basenames identifying Corivo's
/// own windows on platforms that do not provide bundle ids.
pub fn self_executable_names() -> &'static [&'static str] {
    SELF_EXECUTABLE_NAMES
}

/// True when `id` matches any self identifier. macOS bundle ids use exact
/// equality or `<prefix>.` segment boundary; Windows exe basenames use exact
/// case-insensitive equality. Public so other layers (foreground_monitor,
/// snapshot_consumer) can apply the same rule without independently
/// re-implementing platform-specific checks.
pub fn matches_self(id: &str) -> bool {
    let id = id.trim();
    if id.is_empty() {
        return false;
    }

    if SELF_EXECUTABLE_NAMES
        .iter()
        .any(|exe| id.eq_ignore_ascii_case(exe))
    {
        return true;
    }

    SELF_BUNDLE_PREFIXES.iter().any(|prefix| {
        if id == *prefix {
            return true;
        }
        // Require a dot separator so `ai.corivo.desktopclone` doesn't
        // get caught by the `ai.corivo.desktop` prefix.
        id.len() > prefix.len() && id.as_bytes()[prefix.len()] == b'.' && id.starts_with(prefix)
    })
}

/// Stateless engine — cheap to clone (just an Arc-like Vec under the hood
/// is unnecessary; the inner set is small and lookups are O(1) on the
/// HashSet).
#[derive(Debug, Clone)]
pub struct ExclusionEngine {
    blocked_bundle_ids: HashSet<String>,
    blocked_executable_names: HashSet<String>,
}

impl ExclusionEngine {
    /// Engine with only the curated default list active.
    pub fn from_defaults() -> Self {
        Self {
            blocked_bundle_ids: DEFAULT_BLOCKED_BUNDLES
                .iter()
                .map(|s| s.to_string())
                .collect(),
            blocked_executable_names: DEFAULT_BLOCKED_EXECUTABLE_NAMES
                .iter()
                .map(|s| executable_key(s))
                .collect(),
        }
    }

    /// Engine with the default list plus user-supplied additions.
    pub fn with_extras<I: IntoIterator<Item = String>>(extras: I) -> Self {
        let mut set: HashSet<String> = DEFAULT_BLOCKED_BUNDLES
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut exe_set: HashSet<String> = DEFAULT_BLOCKED_EXECUTABLE_NAMES
            .iter()
            .map(|s| executable_key(s))
            .collect();
        for extra in extras {
            let extra = extra.trim();
            if extra.is_empty() {
                continue;
            }
            if is_executable_identifier(extra) {
                exe_set.insert(executable_key(extra));
            } else {
                set.insert(extra.to_string());
            }
        }
        Self {
            blocked_bundle_ids: set,
            blocked_executable_names: exe_set,
        }
    }

    /// Check whether the foreground app should be excluded.
    ///
    /// `None` (no app identifier available) → `Allow` (we can't decide; let it
    /// through). Capture pipeline still has the option to skip on its own
    /// (e.g. idle detection) before reaching the engine.
    pub fn check(&self, bundle_id: Option<&str>) -> ExclusionVerdict {
        let Some(id) = bundle_id else {
            return ExclusionVerdict::Allow;
        };
        // Self-block first: capturing Corivo's own windows just produces
        // blank screenshots, so we drop them with no frame row at all.
        // Always wins over the explicit blocklist.
        if matches_self(id) {
            return ExclusionVerdict::BlockSelf;
        }
        if is_executable_identifier(id)
            && self.blocked_executable_names.contains(&executable_key(id))
        {
            return ExclusionVerdict::Block {
                reason: format!("app:{id}"),
            };
        }
        if self.blocked_bundle_ids.contains(id) {
            ExclusionVerdict::Block {
                reason: format!("app:{id}"),
            }
        } else {
            ExclusionVerdict::Allow
        }
    }

    /// How many app identifiers the engine currently blocks. Mostly for tests.
    pub fn blocked_count(&self) -> usize {
        self.blocked_bundle_ids.len() + self.blocked_executable_names.len()
    }
}

pub fn is_executable_identifier(id: &str) -> bool {
    id.trim().to_ascii_lowercase().ends_with(".exe")
}

pub fn executable_key(id: &str) -> String {
    id.trim().to_ascii_lowercase()
}

impl Default for ExclusionEngine {
    fn default() -> Self {
        Self::from_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_engine_blocks_signal() {
        let engine = ExclusionEngine::from_defaults();
        let verdict = engine.check(Some("org.whispersystems.signal-desktop"));
        match verdict {
            ExclusionVerdict::Block { reason } => {
                assert_eq!(reason, "app:org.whispersystems.signal-desktop");
            }
            _ => panic!("signal should be blocked by default"),
        }
    }

    #[test]
    fn default_engine_blocks_windows_signal() {
        let engine = ExclusionEngine::from_defaults();
        let verdict = engine.check(Some("SIGNAL.EXE"));
        match verdict {
            ExclusionVerdict::Block { reason } => {
                assert_eq!(reason, "app:SIGNAL.EXE");
            }
            _ => panic!("windows signal should be blocked by default"),
        }
    }

    #[test]
    fn default_engine_allows_vscode() {
        let engine = ExclusionEngine::from_defaults();
        assert_eq!(
            engine.check(Some("com.microsoft.VSCode")),
            ExclusionVerdict::Allow
        );
    }

    #[test]
    fn missing_bundle_id_is_allowed() {
        let engine = ExclusionEngine::from_defaults();
        assert_eq!(engine.check(None), ExclusionVerdict::Allow);
    }

    #[test]
    fn user_extras_are_blocked() {
        let engine = ExclusionEngine::with_extras(["com.user.private".to_string()]);
        match engine.check(Some("com.user.private")) {
            ExclusionVerdict::Block { reason } => {
                assert_eq!(reason, "app:com.user.private");
            }
            _ => panic!("user extra should be blocked"),
        }
    }

    #[test]
    fn user_executable_extras_are_case_insensitive() {
        let engine = ExclusionEngine::with_extras(["PrivateVault.exe".to_string()]);
        match engine.check(Some("privatevault.EXE")) {
            ExclusionVerdict::Block { reason } => {
                assert_eq!(reason, "app:privatevault.EXE");
            }
            _ => panic!("user executable extra should be blocked"),
        }
    }

    #[test]
    fn extras_dedupe_with_defaults() {
        // Adding a default-listed id again does not double-count.
        let extras = [
            "org.whispersystems.signal-desktop".to_string(),
            "signal.exe".to_string(),
        ];
        let engine = ExclusionEngine::with_extras(extras);
        let baseline = ExclusionEngine::from_defaults().blocked_count();
        assert_eq!(engine.blocked_count(), baseline);
    }

    #[test]
    fn corivo_itself_is_block_self() {
        let engine = ExclusionEngine::from_defaults();
        assert_eq!(
            engine.check(Some("ai.corivo.desktop")),
            ExclusionVerdict::BlockSelf,
        );
    }

    #[test]
    fn corivo_dev_build_is_block_self() {
        let engine = ExclusionEngine::from_defaults();
        assert_eq!(
            engine.check(Some("ai.corivo.desktop.dev")),
            ExclusionVerdict::BlockSelf,
        );
        assert_eq!(
            engine.check(Some("ai.corivo.desktop.helper")),
            ExclusionVerdict::BlockSelf,
        );
    }

    #[test]
    fn corivo_windows_exe_is_block_self() {
        let engine = ExclusionEngine::from_defaults();
        assert_eq!(
            engine.check(Some("corivo-app.exe")),
            ExclusionVerdict::BlockSelf,
        );
        assert_eq!(
            engine.check(Some("CORIVO-APP.EXE")),
            ExclusionVerdict::BlockSelf,
        );
    }

    #[test]
    fn similar_prefix_does_not_match_self() {
        // No dot separator → not a Corivo bundle id, must NOT be
        // mistaken for one.
        let engine = ExclusionEngine::from_defaults();
        assert_eq!(
            engine.check(Some("ai.corivo.desktopclone")),
            ExclusionVerdict::Allow,
        );
        assert_eq!(
            engine.check(Some("corivo-app-helper.exe")),
            ExclusionVerdict::Allow,
        );
        assert_eq!(
            engine.check(Some("SignalHelper.exe")),
            ExclusionVerdict::Allow,
        );
    }
}
