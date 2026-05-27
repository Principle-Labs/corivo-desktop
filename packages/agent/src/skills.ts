// Load Agent Skills (https://agentskills.io) from four locations and
// merge them into a single list the system prompt can advertise:
//
//   1. `~/.agents/skills/`         — corivo's user-scope skill dir.
//      Loaded via the higher-level `loadSkills` so it picks up
//      project-local skills too if we ever introduce a workspace
//      concept (today cwd=homedir, so the project hook is a no-op).
//   2. `bundledSkillsDir`         — app-private skills shipped inside
//      Corivo.app via Tauri `bundle.resources`. Rust passes this in
//      via `SidecarInput.bundled_skills_dir`. Read-only on disk.
//   3. `~/.claude/skills/`        — the user's existing Claude Code
//      skill pool. Loaded via the simpler `loadSkillsFromDir` because
//      we don't want includeDefaults' implicit `.pi/skills` lookup to
//      leak in here.
//   4. `~/.corivo/skills/market/` — skills the user installed from the
//      Corivo skill market. Read directly here so a market install is
//      visible to this agent immediately, without depending on the
//      Rust-side `SkillShareService::sync` symlink bridge (which is
//      unix-only and targets the bundled `claude` CLI's config dir,
//      not this loader).
//
// Skills with the same `name` collide. Precedence: user `~/.agents` >
// bundled > `~/.claude` > market. The user's local override stays
// strongest; bundled shadows `~/.claude` so a host-wide skill named
// `corivo-feedback` (or similar) can't replace the version we ship;
// market is last so a hand-edited override under `~/.agents` or
// `~/.claude` still wins over whatever the published version is.

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
 * Resolve the host-side skill roots. Exported so tests / callers can
 * inspect which paths were probed.
 */
export interface SkillRoots {
  /** `~/.agents` — corivo agent config dir (sibling of `~/.claude`, `~/.pi`). */
  agentConfigDir: string;
  /** `~/.claude/skills` — Claude Code's user-skills directory. */
  claudeSkillsDir: string;
  /** `~/.corivo/skills/market` — skills installed from the Corivo skill market. */
  marketSkillsDir: string;
}

export function resolveSkillRoots(): SkillRoots {
  const home = os.homedir();
  return {
    agentConfigDir: path.join(home, ".agents"),
    claudeSkillsDir: path.join(home, ".claude", "skills"),
    marketSkillsDir: path.join(home, ".corivo", "skills", "market"),
  };
}

export interface LoadCorivoSkillsOptions {
  /** App-private bundled skills root resolved by Rust. When omitted
   *  (mock-mode, missing resource), no bundled skills are loaded. */
  bundledSkillsDir?: string;
  /**
   * Snapshot of `Config.exec_agent.skill_share.enabled` —
   * `SidecarInput.enabled_skills`. When `undefined`, no filter is
   * applied (legacy callers, mock-mode). When an array — including
   * the empty array — only skills whose `name` is present pass through.
   *
   * Filtering happens AFTER the four-source merge so that a skill the
   * user disabled in Settings stays hidden even if it ships from the
   * higher-precedence bundled / agent sources.
   */
  enabledSkills?: string[];
}

/**
 * Load skills from `~/.agents/skills/`, the bundled-skills resource dir,
 * `~/.claude/skills/`, and `~/.corivo/skills/market/`; merge by name
 * with precedence agent > bundled > claude > market. Returns the
 * unified list plus all validation diagnostics. Diagnostics are also
 * logged via `log.warn` so they surface in the daily log.
 *
 * If `enabledSkills` is supplied the merged list is filtered down to
 * those names — see `LoadCorivoSkillsOptions.enabledSkills`.
 */
export function loadCorivoSkills(
  opts: LoadCorivoSkillsOptions = {},
): LoadSkillsResult {
  const { agentConfigDir, claudeSkillsDir, marketSkillsDir } =
    resolveSkillRoots();
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

  const marketRes = loadSkillsFromDir({
    dir: marketSkillsDir,
    source: "market",
  });

  // Merge with precedence: agent > bundled > claude > market.
  // Collisions surface as diagnostics so a user who unintentionally
  // shadows a bundled skill (e.g. by dropping
  // `~/.agents/skills/corivo-feedback/`) can see why their override is
  // taking effect — and a host-wide ~/.claude skill that gets
  // overridden by something we ship is also visible.
  const merged = new Map<string, Skill>();
  const diagnostics: ResourceDiagnostic[] = [
    ...agentRes.diagnostics,
    ...bundledRes.diagnostics,
    ...claudeRes.diagnostics,
    ...marketRes.diagnostics,
  ];
  for (const skill of agentRes.skills) {
    merged.set(skill.name, skill);
  }
  for (const skill of bundledRes.skills) {
    const existing = merged.get(skill.name);
    if (existing) {
      diagnostics.push({
        type: "collision",
        message: `bundled skill "${skill.name}" shadowed by ~/.agents/skills`,
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
  for (const skill of marketRes.skills) {
    const existing = merged.get(skill.name);
    if (existing) {
      diagnostics.push({
        type: "collision",
        message: `market skill "${skill.name}" shadowed by a higher-precedence source`,
        path: skill.filePath,
        collision: {
          resourceType: "skill",
          name: skill.name,
          winnerPath: existing.filePath,
          loserPath: skill.filePath,
          winnerSource: "agent",
          loserSource: "market",
        },
      });
      continue;
    }
    merged.set(skill.name, skill);
  }

  const mergedSkills = [...merged.values()];

  // Apply the Settings → 技能 allow-list AFTER merging. We do it here
  // rather than per-source so that a user-disabled skill stays hidden
  // even when it has higher-precedence variants — the toggle reflects
  // user intent on the *name*, not on a particular source. `undefined`
  // means the caller didn't pass the snapshot (e.g. tests / mock-mode),
  // in which case we preserve the legacy "no filter" behavior.
  const skills =
    opts.enabledSkills === undefined
      ? mergedSkills
      : (() => {
          const allow = new Set(opts.enabledSkills);
          return mergedSkills.filter((s) => allow.has(s.name));
        })();

  log.info("agent.skills.loaded", {
    agent_config_dir: agentConfigDir,
    bundled_skills_dir: bundledDir ?? null,
    claude_skills_dir: claudeSkillsDir,
    market_skills_dir: marketSkillsDir,
    agent_skill_count: agentRes.skills.length,
    bundled_skill_count: bundledRes.skills.length,
    claude_skill_count: claudeRes.skills.length,
    market_skill_count: marketRes.skills.length,
    merged_count: mergedSkills.length,
    enabled_filter:
      opts.enabledSkills === undefined ? null : opts.enabledSkills.length,
    skills_after_filter: skills.length,
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
