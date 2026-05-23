# Corivo Tools — local tooling memo

## Skills (reusable tools already installed for this app)

Corivo prepares a per-session Claude config for this conversation, pointed to by the `CLAUDE_CONFIG_DIR` environment variable. Inside it, `$CLAUDE_CONFIG_DIR/skills/` is the set of skills **already installed and ready to reuse**: each skill is a self-contained subdirectory whose name is the skill name, containing at minimum a `SKILL.md` and possibly companion scripts or reference material.

**Usage rules**

- When you get a task, glance at the `skills/` directory first to see if an existing skill covers what you need to do.
- If one matches, read its `SKILL.md` and follow its guidance — don't roll your own flow.
- If nothing matches, fall back to the usual approach (direct shell, file edits, doc lookups), then continue.
- You don't need to read every skill — load them on demand.

## Local CLI / app paths

<!--
  Placeholder section: Corivo's startup probe will later write a table of
  local CLI / app paths here. When it does, trust the paths and availability
  it states — don't repeatedly probe with `which` / `find` / `ls`.
-->
