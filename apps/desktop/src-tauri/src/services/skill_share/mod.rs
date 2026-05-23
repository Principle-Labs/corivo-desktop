//! Bridge host-installed Claude/Agents skills into the Corivo-isolated
//! `CLAUDE_CONFIG_DIR/skills/` so the bundled `claude` can see the
//! user's lark / yansu / superpowers / … skill collection without
//! polluting the wider isolation contract.
//!
//! Sources scanned (in priority order; first wins on name collision):
//!   1. `~/.agents/skills/`   — single-source-of-truth in airbo's setup
//!   2. `~/.claude/skills/`   — Claude Code's built-in skill dir
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
}

impl SkillSource {
    fn label(self) -> &'static str {
        match self {
            Self::Agents => "agents",
            Self::Claude => "claude",
        }
    }

    /// Classification by source directory — the "我的工作流" route
    /// filters on this. All current sources are external tool
    /// collections (lark cli, Claude Code), i.e. `Capability`. Once
    /// Corivo owns a writable skills dir (e.g. `$APPDATA/corivo/skills/`)
    /// for agent-crystalized routines, those entries will be `Workflow`.
    fn kind(self) -> SkillKind {
        match self {
            Self::Agents | Self::Claude => SkillKind::Capability,
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
    ///   - Each enabled skill that the host still provides → symlink.
    ///   - Existing symlinks not in `enabled` (or whose source vanished)
    ///     → removed.
    ///   - Non-symlink entries are preserved untouched (defensive: we
    ///     never delete a real directory we didn't create).
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

        // 1. Walk current dst entries, drop any symlink we no longer
        //    want (not enabled, or source vanished, or pointed
        //    elsewhere than we'd point now).
        if let Ok(entries) = fs::read_dir(&dst_root) {
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(meta) = fs::symlink_metadata(&path) else {
                    continue;
                };
                if !meta.file_type().is_symlink() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                let want = enabled.contains(&name)
                    && by_name.get(&name).is_some_and(|skill| {
                        fs::read_link(&path).is_ok_and(|cur| cur == skill.path)
                    });
                if !want {
                    let _ = fs::remove_file(&path);
                }
            }
        }

        // 2. Create any missing enabled symlinks. Skills enabled but
        //    not currently on host stay missing (silent — they pop back
        //    when the host reinstalls).
        for name in enabled {
            let Some(skill) = by_name.get(name) else {
                continue;
            };
            let dst = dst_root.join(name);
            if dst.exists() || fs::symlink_metadata(&dst).is_ok() {
                // Already correct or someone else owns this slot — covered
                // by the cleanup pass above.
                continue;
            }
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(&skill.path, &dst).map_err(|e| {
                    CorivoError::Internal(format!(
                        "skill_share: symlink {} → {} 失败：{e}",
                        dst.display(),
                        skill.path.display()
                    ))
                })?;
            }
            #[cfg(not(unix))]
            {
                let _ = skill;
                return Err(CorivoError::Internal(
                    "skill_share: symlink bridging is not yet supported on this platform"
                        .to_string(),
                ));
            }
        }

        Ok(())
    }
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
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
