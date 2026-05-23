//! Tauri commands for the skill-share Settings UI.
//!
//! Read-only — the actual mutation surface (which skills are enabled)
//! goes through `set_config` because the list lives on
//! `ExecAgentConfig::skill_share::enabled`. `services::skill_share::sync`
//! is then driven by the config-write hook in `commands::config::set_config`
//! and at boot in `lib.rs`.

use serde::Serialize;
use tauri::State;
use ts_rs::TS;

use crate::commands::config::AppState;
use crate::domain::ipc_error::TauriError;
use crate::services::skill_share::{DiscoveredSkill, SkillKind, SkillSource};

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "camelCase")]
pub struct AvailableSkill {
    /// Top-level directory name; also the key used in
    /// `ExecAgentConfig.skill_share.enabled`.
    pub name: String,
    /// What this entry represents — a user-facing workflow or an
    /// external capability. The 工作流 route filters on `workflow`;
    /// `capability` entries stay in Settings → 技能.
    pub kind: AvailableSkillKind,
    /// Which host source contributed this entry. When the same name
    /// appears in both, `~/.agents/skills/` wins and `claude` is hidden.
    pub source: AvailableSkillSource,
    /// Absolute path to the host directory holding `SKILL.md`.
    /// Surfaced for UX ("hover to see source") + diagnostics; the UI
    /// must not edit it.
    pub path: String,
    /// Best-effort `description:` from `SKILL.md` frontmatter.
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum AvailableSkillSource {
    Agents,
    Claude,
}

#[derive(Debug, Clone, Copy, Serialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum AvailableSkillKind {
    Workflow,
    Capability,
}

impl From<SkillSource> for AvailableSkillSource {
    fn from(s: SkillSource) -> Self {
        match s {
            SkillSource::Agents => Self::Agents,
            SkillSource::Claude => Self::Claude,
        }
    }
}

impl From<SkillKind> for AvailableSkillKind {
    fn from(k: SkillKind) -> Self {
        match k {
            SkillKind::Workflow => Self::Workflow,
            SkillKind::Capability => Self::Capability,
        }
    }
}

impl From<DiscoveredSkill> for AvailableSkill {
    fn from(s: DiscoveredSkill) -> Self {
        Self {
            name: s.name,
            kind: s.kind.into(),
            source: s.source.into(),
            path: s.path.to_string_lossy().into_owned(),
            description: s.description,
        }
    }
}

#[tauri::command]
pub async fn skills_list_available(
    state: State<'_, AppState>,
) -> Result<Vec<AvailableSkill>, TauriError> {
    let service = state
        .skill_share
        .as_ref()
        .ok_or_else(|| TauriError::Unknown {
            message: "skill_share service is not ready".to_string(),
        })?
        .clone();
    Ok(service
        .scan()
        .into_iter()
        .map(AvailableSkill::from)
        .collect())
}
