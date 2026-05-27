//! Bridge host-installed Claude/Agents skills into the Corivo-isolated
//! `CLAUDE_CONFIG_DIR/skills/` so the bundled `claude` can see the
//! user's lark / yansu / superpowers / … skill collection without
//! polluting the wider isolation contract.
//!
//! Sources scanned (in priority order; first wins on name collision):
//!   1. `~/.agents/skills/`         — single-source-of-truth in airbo's setup
//!   2. `~/.claude/skills/`         — Claude Code's built-in skill dir
//!   3. `~/.corivo/skills/market/`  — installed from the Corivo skill market
//!                                    (commands::market writes here)
//!
//! De-duplication is by skill **name** (= top-level directory name).
//! When the same name resolves to the same canonical path through both
//! sources (because `~/.claude/skills/foo` is just a symlink to
//! `~/.agents/skills/foo`) we still surface it only once.
//!
//! Sync writes per-skill symlinks: `$APPDATA/claude-config/skills/<name>
//! → <host source path>`. Stale links (skill no longer enabled, or no
//! longer present on host) are removed. Non-symlink entries already
//! living under the dest dir (e.g. a Corivo-bundled skill we ship in
//! the future) are left alone.
//!
//! Idempotent — `sync` can be called liberally (boot + every config
//! write).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{CorivoError, Result};

const CLAUDE_CONFIG_SUBDIR: &str = "claude-config";
const SKILLS_SUBDIR: &str = "skills";
const SKILL_MANIFEST: &str = "SKILL.md";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    Agents,
    Claude,
    /// Installed by user from the Corivo skill market (see
    /// `commands::market::skill_market_install`). Directory layout under
    /// `~/.corivo/skills/market/<slug>/`; each install has a
    /// `.market-meta.json` sidecar recording the source commit SHA.
    Market,
}

impl SkillSource {
    fn label(self) -> &'static str {
        match self {
            Self::Agents => "agents",
            Self::Claude => "claude",
            Self::Market => "market",
        }
    }

    /// Classification by source directory — the "我的工作流" route
    /// filters on this. All current sources surface as `Capability` —
    /// market skills are typically external-tool style today; if/when
    /// users start publishing workflow-kind skills, this will fan out
    /// based on SKILL.md frontmatter rather than source alone.
    fn kind(self) -> SkillKind {
        match self {
            Self::Agents | Self::Claude | Self::Market => SkillKind::Capability,
        }
    }
}

/// What this skill represents to the user.
///
/// - `Workflow` — user-authored or agent-crystalized procedural memory;
///   shown on the "我的工作流" route.
/// - `Capability` — primitive provided by an external MCP/agent toolset
///   (lark, Claude Code, etc.); not shown on the workflows route — the
///   user manages exposure under Settings → 技能.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillKind {
    Workflow,
    Capability,
}

#[derive(Debug, Clone)]
pub struct DiscoveredSkill {
    pub name: String,
    pub source: SkillSource,
    pub kind: SkillKind,
    /// The directory that holds `SKILL.md`. May be a symlink — we
    /// symlink to it as-is rather than to its canonical target so
    /// e.g. cleanup elsewhere on the host stays observable.
    pub path: PathBuf,
    /// Skill description from SKILL.md frontmatter, if parseable.
    pub description: Option<String>,
}

pub struct SkillShareService {
    app_data_dir: PathBuf,
    home_dir: Option<PathBuf>,
}

impl SkillShareService {
    pub fn new(app_data_dir: PathBuf) -> Self {
        Self {
            app_data_dir,
            home_dir: dirs_home(),
        }
    }

    /// Used by the `skills_list_available` Tauri command. Walks the
    /// host source dirs and returns the de-duplicated skill list in
    /// stable name-sorted order so the Settings UI doesn't flicker.
    pub fn scan(&self) -> Vec<DiscoveredSkill> {
        let Some(home) = &self.home_dir else {
            return Vec::new();
        };
        let mut by_name: HashMap<String, DiscoveredSkill> = HashMap::new();
        for (subdir, source) in [
            (".agents/skills", SkillSource::Agents),
            (".claude/skills", SkillSource::Claude),
            (".corivo/skills/market", SkillSource::Market),
        ] {
            let root = home.join(subdir);
            for skill in scan_dir(&root, source) {
                by_name.entry(skill.name.clone()).or_insert(skill);
            }
        }
        let mut out: Vec<DiscoveredSkill> = by_name.into_values().collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Reconcile `$APPDATA/claude-config/skills/` with `enabled`:
    ///   - Each enabled skill that the host still provides → managed
    ///     link (symlink on Unix, NTFS junction on Windows). Junctions
    ///     are used on Windows because real symlinks require admin /
    ///     developer mode while junctions don't.
    ///   - Existing managed links not in `enabled` (or whose source
    ///     vanished, or pointing elsewhere than we'd point now)
    ///     → removed.
    ///   - Non-link entries (real dirs, files, foreign reparse points)
    ///     are preserved untouched: defensive — we never delete
    ///     anything we didn't create.
    pub fn sync(&self, enabled: &[String]) -> Result<()> {
        let dst_root = self
            .app_data_dir
            .join(CLAUDE_CONFIG_SUBDIR)
            .join(SKILLS_SUBDIR);
        fs::create_dir_all(&dst_root).map_err(|e| {
            CorivoError::Internal(format!(
                "skill_share: 创建 {} 失败：{e}",
                dst_root.display()
            ))
        })?;

        let available = self.scan();
        let by_name: HashMap<String, &DiscoveredSkill> =
            available.iter().map(|s| (s.name.clone(), s)).collect();

        // 1. Walk current dst entries, drop any managed link we no
        //    longer want (not enabled, source vanished, or pointing
        //    elsewhere than we'd point now). Non-link entries are
        //    skipped.
        if let Ok(entries) = fs::read_dir(&dst_root) {
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(current_target) = read_managed_link(&path) else {
                    continue;
                };
                let name = entry.file_name().to_string_lossy().to_string();
                let want = enabled.contains(&name)
                    && by_name.get(&name).is_some_and(|skill| {
                        link_target_matches(&current_target, &skill.path)
                    });
                if !want {
                    let _ = remove_managed_link(&path);
                }
            }
        }

        // 2. Create any missing enabled links. Skills enabled but not
        //    currently on host stay missing (silent — they pop back
        //    when the host reinstalls).
        for name in enabled {
            let Some(skill) = by_name.get(name) else {
                continue;
            };
            let dst = dst_root.join(name);
            if dst.exists() || fs::symlink_metadata(&dst).is_ok() {
                // Already correct or someone else owns this slot —
                // covered by the cleanup pass above.
                continue;
            }
            create_managed_link(&skill.path, &dst).map_err(|e| {
                // Windows junctions require src + dst to live on the
                // same NTFS volume — surface a hint so a cross-drive
                // setup doesn't read as a mysterious OS error.
                #[cfg(windows)]
                let hint =
                    " (Windows junctions require source and destination on the same NTFS volume)";
                #[cfg(not(windows))]
                let hint = "";
                CorivoError::Internal(format!(
                    "skill_share: link {} → {} 失败：{e}{hint}",
                    dst.display(),
                    skill.path.display()
                ))
            })?;
        }

        Ok(())
    }
}

fn dirs_home() -> Option<PathBuf> {
    // `HOME` is the canonical env var on Unix-likes; `USERPROFILE` is
    // Windows' default — `HOME` is not normally set on Windows and we
    // were silently returning `None`, which made every skill scan a
    // no-op (sync would then wipe nothing and create nothing).
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Read the target of a "managed link" — a symlink on Unix or an NTFS
/// junction on Windows. Returns `None` when `path` is not such a link
/// (real directory, regular file, missing, or any reparse-point flavor
/// we don't manage — e.g. a foreign symlink the user dropped in).
///
/// `SkillShareService::sync` uses this to distinguish links we created
/// (and may rewrite/delete) from anything else under the dst dir.
fn read_managed_link(path: &Path) -> Option<PathBuf> {
    #[cfg(unix)]
    {
        let meta = fs::symlink_metadata(path).ok()?;
        if !meta.file_type().is_symlink() {
            return None;
        }
        fs::read_link(path).ok()
    }
    #[cfg(windows)]
    {
        // `junction::exists` returns true only for NTFS junctions — not
        // symlinks, not regular dirs. Anything that isn't a junction we
        // leave alone (we never created it).
        match junction::exists(path) {
            Ok(true) => junction::get_target(path).ok(),
            _ => None,
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        None
    }
}

/// Remove a managed link previously created by `create_managed_link`.
/// On Windows, junctions live in the namespace as directories — plain
/// `fs::remove_file` fails with "Access is denied". `junction::delete`
/// tears down the reparse point without recursing into the target.
fn remove_managed_link(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        fs::remove_file(path)
    }
    #[cfg(windows)]
    {
        junction::delete(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "managed link removal not supported on this platform",
        ))
    }
}

/// Create a managed link `dst → src` (i.e. `dst` is the new entry that
/// will resolve to the contents of `src`). Symlink on Unix, NTFS
/// junction on Windows.
fn create_managed_link(src: &Path, dst: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(src, dst)
    }
    #[cfg(windows)]
    {
        junction::create(src, dst)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (src, dst);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "managed link creation not supported on this platform",
        ))
    }
}

/// Check whether an existing link's `current` target points at the same
/// place as `expected`. Fast path is raw byte equality (preserves the
/// pre-Windows-port behavior on Unix). Slow path canonicalizes both
/// sides — necessary on Windows because `junction::get_target` returns
/// a normalized absolute Win32 path that may not byte-match the original
/// `expected` we passed to `create_managed_link`. When either side
/// fails to canonicalize, treat as mismatch: the cleanup pass will
/// remove + recreate on the next sync, which is idempotent.
fn link_target_matches(current: &Path, expected: &Path) -> bool {
    if current == expected {
        return true;
    }
    match (fs::canonicalize(current), fs::canonicalize(expected)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
}

fn scan_dir(root: &Path, source: SkillSource) -> Vec<DiscoveredSkill> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        // Follow symlinks (~/.claude/skills/* are usually links to
        // ~/.agents/skills/*) when checking "is it a directory with
        // SKILL.md?".
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        if !meta.is_dir() {
            continue;
        }
        let manifest = path.join(SKILL_MANIFEST);
        if !manifest.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let description = read_skill_description(&manifest);
        tracing::debug!(
            source = source.label(),
            name = %name,
            path = %path.display(),
            "skill_share.discovered"
        );
        out.push(DiscoveredSkill {
            name,
            source,
            kind: source.kind(),
            path,
            description,
        });
    }
    out
}

/// Best-effort frontmatter `description:` extraction. Avoids pulling in
/// a YAML crate just for one field — frontmatter is a `---`-fenced block
/// at the top of the file and we only care about a single key.
fn read_skill_description(manifest: &Path) -> Option<String> {
    let text = fs::read_to_string(manifest).ok()?;
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut buf = String::new();
    let mut in_description = false;
    for line in lines {
        let trimmed = line.trim_end();
        if trimmed == "---" {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("description:") {
            in_description = true;
            buf.push_str(rest.trim().trim_matches('"').trim_matches('\''));
        } else if in_description {
            // Continue a YAML folded / multi-line description until the
            // next top-level key (`key:` at column 0). Anything indented
            // is treated as continuation.
            if !trimmed.is_empty()
                && !trimmed.starts_with(char::is_whitespace)
                && trimmed.contains(':')
            {
                break;
            }
            if !buf.is_empty() {
                buf.push(' ');
            }
            buf.push_str(trimmed.trim());
        }
    }
    let cleaned = buf.trim().to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}
