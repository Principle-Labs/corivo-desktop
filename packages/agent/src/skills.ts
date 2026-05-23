// Load Agent Skills (https://agentskills.io) from three locations and merge
// them into a single list the system prompt can advertise:
//
//   1. `~/.agent/skills/`        — corivo's user-scope skill dir. Loaded
//      via the higher-level `loadSkills` so it picks up project-local
//      skills too if we ever introduce a workspace concept (today
//      cwd=homedir, so the project hook is a no-op).
//   2. `bundledSkillsDir`        — app-private skills shipped inside
//      Corivo.app via Tauri `bundle.resources`. Rust passes this in
//      via `SidecarInput.bundled_skills_dir`. Read-only on disk.
//   3. `~/.claude/skills/`       — the user's existing Claude Code skill
//      pool. Loaded via the simpler `loadSkillsFromDir` because we
//      don't want includeDefaults' implicit `.pi/skills` lookup to
//      leak in here.
//
// Skills with the same `name` collide. Precedence: user `~/.agent` wins
// over bundled, and bundled wins over `~/.claude`. The user's local
// override is intentional — they edited it for a reason. Bundled
// shadows ~/.claude so a host-wide skill named `corivo-feedback` (or
// similar) can't replace the version we ship.

import os from "node:os";
import path from "node:path";

import {
  loadSkills,
  loadSkillsFromDir,
  type LoadSkillsResult,
  type ResourceDiagnostic,
  type Skill,
} from "@mariozechner/pi-coding-agent";

import { log } from "./log.js";

/**
 * Resolve the two skill roots. Exported so tests / callers can inspect
 * which paths were probed.
 */
export interface SkillRoots {
  /** `~/.agent` — corivo agent config dir (sibling of `~/.claude`, `~/.pi`). */
  agentConfigDir: string;
  /** `~/.claude/skills` — Claude Code's user-skills directory. */
  claudeSkillsDir: string;
}

export function resolveSkillRoots(): SkillRoots {
  const home = os.homedir();
  return {
    agentConfigDir: path.join(home, ".agent"),
    claudeSkillsDir: path.join(home, ".claude", "skills"),
  };
}

export interface LoadCorivoSkillsOptions {
  /** App-private bundled skills root resolved by Rust. When omitted
   *  (mock-mode, missing resource), no bundled skills are loaded. */
  bundledSkillsDir?: string;
}

/**
 * Load skills from `~/.agent/skills/`, the bundled-skills resource dir,
 * and `~/.claude/skills/`; merge by name with precedence agent >
 * bundled > claude. Returns the unified list plus all validation
 * diagnostics. Diagnostics are also logged via `log.warn` so they
 * surface in the daily log.
 */
export function loadCorivoSkills(
  opts: LoadCorivoSkillsOptions = {},
): LoadSkillsResult {
  const { agentConfigDir, claudeSkillsDir } = resolveSkillRoots();
  const bundledDir = opts.bundledSkillsDir;

  // `loadSkills` honors `agentDir` for the user-scope dir and `cwd` for
  // a project-scope `.pi/skills`. We have no workspace concept yet, so
  // pass cwd=homedir; the project lookup will be a missing-dir no-op.
  const agentRes = loadSkills({
    cwd: os.homedir(),
    agentDir: agentConfigDir,
    skillPaths: [],
    includeDefaults: true,
  });

  const bundledRes = bundledDir
    ? loadSkillsFromDir({ dir: bundledDir, source: "agent-bundled" })
    : { skills: [] as Skill[], diagnostics: [] as ResourceDiagnostic[] };

  const claudeRes = loadSkillsFromDir({
    dir: claudeSkillsDir,
    source: "claude",
  });

  // Merge with precedence: agent > bundled > claude. Collisions surface
  // as diagnostics so a user who unintentionally shadows a bundled
  // skill (e.g. by dropping `~/.agent/skills/corivo-feedback/`) can
  // see why their override is taking effect — and a host-wide
  // ~/.claude skill that gets overridden by something we ship is also
  // visible.
  const merged = new Map<string, Skill>();
  const diagnostics: ResourceDiagnostic[] = [
    ...agentRes.diagnostics,
    ...bundledRes.diagnostics,
    ...claudeRes.diagnostics,
  ];
  for (const skill of agentRes.skills) {
    merged.set(skill.name, skill);
  }
  for (const skill of bundledRes.skills) {
    const existing = merged.get(skill.name);
    if (existing) {
      diagnostics.push({
        type: "collision",
        message: `bundled skill "${skill.name}" shadowed by ~/.agent/skills`,
        path: skill.filePath,
        collision: {
          resourceType: "skill",
          name: skill.name,
          winnerPath: existing.filePath,
          loserPath: skill.filePath,
          winnerSource: "agent",
          loserSource: "agent-bundled",
        },
      });
      continue;
    }
    merged.set(skill.name, skill);
  }
  for (const skill of claudeRes.skills) {
    const existing = merged.get(skill.name);
    if (existing) {
      diagnostics.push({
        type: "collision",
        message: `claude skill "${skill.name}" shadowed by corivo skill source`,
        path: skill.filePath,
        collision: {
          resourceType: "skill",
          name: skill.name,
          winnerPath: existing.filePath,
          loserPath: skill.filePath,
          winnerSource: "agent",
          loserSource: "claude",
        },
      });
      continue;
    }
    merged.set(skill.name, skill);
  }

  const skills = [...merged.values()];

  log.info("agent.skills.loaded", {
    agent_config_dir: agentConfigDir,
    bundled_skills_dir: bundledDir ?? null,
    claude_skills_dir: claudeSkillsDir,
    agent_skill_count: agentRes.skills.length,
    bundled_skill_count: bundledRes.skills.length,
    claude_skill_count: claudeRes.skills.length,
    merged_count: skills.length,
    diagnostic_count: diagnostics.length,
  });

  for (const d of diagnostics) {
    if (d.type === "error") {
      log.error("agent.skills.error", { message: d.message, path: d.path });
    } else if (d.type === "warning") {
      log.warn("agent.skills.warning", { message: d.message, path: d.path });
    } else if (d.type === "collision") {
      log.warn("agent.skills.collision", {
        message: d.message,
        winner: d.collision?.winnerPath,
        loser: d.collision?.loserPath,
      });
    }
  }

  return { skills, diagnostics };
}
