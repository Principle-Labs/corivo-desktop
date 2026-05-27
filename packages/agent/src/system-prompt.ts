// Build the agent's system prompt. Phase A keeps this minimal; Phase C will
// flesh out the Quick Ask vs /ask flavor distinctions and wire focus_context
// into the prompt.

import { formatSkillsForPrompt } from "@mariozechner/pi-coding-agent";

import type { ConnectorsInput, SidecarInput } from "./types.js";
import { loadCorivoSkills } from "./skills.js";

// Host-platform sentence injected into the system prompt. On Windows the
// `bash` tool actually invokes PowerShell 7 (pwsh.exe) or Windows
// PowerShell 5 (powershell.exe) — see packages/agent/src/native-tools/index.ts.
// Tell the model up-front so it writes platform-native shell idioms
// instead of leaning on POSIX assumptions ($HOME, `which`, `&&`-chain
// semantics, here-strings, …) that don't survive the translation.
function describeHostShell(): { intro: string; tool: string } {
  if (process.platform === "win32") {
    return {
      intro:
        "You run on the user's Windows machine. Your voice, working principles, and local\n" +
        "tooling context are appended below — read those for who you are and how to talk.",
      tool:
        "- bash: run shell commands on the user's machine (cwd defaults to %USERPROFILE%). " +
        "The bash tool actually invokes PowerShell 7 (pwsh.exe) when available, otherwise " +
        "Windows PowerShell 5 (powershell.exe) — write PowerShell-flavored commands " +
        "(`Get-ChildItem`, `Set-Location`, `$env:VAR`, …) and use semicolons or `;` chains " +
        "instead of `&&`. Standard POSIX aliases like `ls`, `pwd`, `cat`, `cp`, `mv`, `rm` " +
        "still work because PowerShell defines them as cmdlet aliases.",
    };
  }
  if (process.platform === "darwin") {
    return {
      intro:
        "You run on the user's macOS machine. Your voice, working principles, and local\n" +
        "tooling context are appended below — read those for who you are and how to talk.",
      tool: "- bash: run shell commands on the user's machine (cwd defaults to $HOME).",
    };
  }
  // Linux / other Unix: same shape as macOS, different label.
  return {
    intro:
      `You run on the user's ${process.platform} machine. Your voice, working principles, and local\n` +
      "tooling context are appended below — read those for who you are and how to talk.",
    tool: "- bash: run shell commands on the user's machine (cwd defaults to $HOME).",
  };
}

const HOST_SHELL = describeHostShell();

const BASE_SYSTEM_PROMPT = `${HOST_SHELL.intro}

Tools available to you:
- recall_screen_history: search what the user has seen on screen recently.
- ask_permission: ask the user to confirm a sensitive or destructive action.
${HOST_SHELL.tool}
- read, write, edit: read / create / modify files.
- grep, find, ls: search file contents, locate files by name, list directories.

Use bash and the file tools to actually do work for the user — don't just describe
what they could run themselves. When a command is destructive or touches secrets,
call ask_permission first.`.trim();

/**
 * Hand-written per-connector pointer block. The base system prompt
 * statically enumerates the built-in tools — without an equivalent
 * mention here, the model strongly biases toward `bash` (it's the
 * generic "do anything" tool the prompt explicitly endorses) and
 * never reaches for connector tools that are technically in its
 * tool registry but absent from the prompt's introduction.
 *
 * Keep entries short + explicitly redirect away from bash fallbacks.
 * Adding a new connector here = one row; the loader/registry layers
 * already handle availability gating.
 */
const CONNECTOR_HINTS: Record<string, (accountEmail: string | null) => string> = {
  gcal: (email) =>
    `- Google Calendar${email ? ` (${email})` : ""}: call \`gcal_list_events\` to read the user's schedule, ` +
    "`gcal_create_event` to add an event with structured fields (when you already have start/end), or " +
    '`gcal_quick_add` for natural-language inputs like "lunch with Alex Friday noon". ' +
    "Do NOT fall back to bash, `open` calendar URLs, or asking the user to add events by hand — " +
    "these tools write to the calendar directly.",
  gmail: (email) =>
    `- Gmail${email ? ` (${email})` : ""}: call \`gmail_send\` to send an email through the user's connected Gmail account. ` +
    "Do NOT fall back to bash, sendmail, or opening a compose URL in the browser — the Gmail API tool sends directly.",
  "google-docs": (email) =>
    `- Google Docs${email ? ` (${email})` : ""}: use \`google_docs_create\` to make a new doc, ` +
    "`google_docs_read` to fetch its plain text, `google_docs_append` to add content at the end, " +
    "and `google_docs_replace_text` for find-and-replace. Accepts either a docs.google.com URL or " +
    "the bare documentId — the tools parse the id out. Do NOT fall back to bash, `curl`, or " +
    "opening docs.google.com in a browser; the Docs API tools edit directly.",
  slack: (email) =>
    `- Slack${email ? ` (${email})` : ""}: call \`slack_send_message\` to send a message as the user ` +
    "(accepts a channel id or a `#channel` / bare name — resolved automatically). " +
    "Use `slack_list_channels` to discover channel ids first when the user names a channel you " +
    "don't already know, and `slack_search_messages` to find prior conversations (Slack query " +
    "syntax: `in:#channel`, `from:@user`, `before:YYYY-MM-DD`, …). Do NOT fall back to bash, " +
    "`curl`, or opening slack.com in a browser; the Slack API tools post and read directly.",
};

function buildConnectorsBlock(connectors: ConnectorsInput | undefined): string {
  if (!connectors || connectors.enabled.length === 0) {
    return "";
  }
  const lines: string[] = [];
  for (const c of connectors.enabled) {
    const hint = CONNECTOR_HINTS[c.id];
    if (hint) {
      lines.push(hint(c.account_email));
    }
  }
  if (lines.length === 0) {
    return "";
  }
  return [
    "",
    "",
    "Connected third-party services (use the connector tools FIRST when the user asks about these — do not improvise with bash):",
    ...lines,
  ].join("\n");
}

export function buildSystemPrompt(input: SidecarInput): string {
  const isQuickAsk = !!input.focus_context;
  const flavor = isQuickAsk
    ? "\n\nMode: Quick Ask. The user's current screen context is provided below."
    : "\n\nMode: /ask thread. Multi-turn conversation.";

  let prompt = BASE_SYSTEM_PROMPT + buildConnectorsBlock(input.connectors) + flavor;

  if (input.focus_context) {
    const fc = input.focus_context;
    const parts: string[] = [];
    if (fc.summary) parts.push(`Summary: ${fc.summary}`);
    if (fc.primary_text) parts.push(`Primary text:\n${fc.primary_text}`);
    if (fc.selection) parts.push(`User selection: ${fc.selection}`);
    if (parts.length > 0) {
      prompt += `\n\n<focus_context>\n${parts.join("\n\n")}\n</focus_context>`;
    }
  }

  if (input.system_prompt_extra) {
    prompt += `\n\n${input.system_prompt_extra}`;
  }

  // Agent Skills — merge of ~/.agents/skills, the app-private
  // bundled-skills resource (passed in via SidecarInput), and
  // ~/.claude/skills. `formatSkillsForPrompt` returns "" when no
  // skills are visible (empty list, or all are
  // disable-model-invocation); concat is safe in that case. Skills
  // showing up here means the model can `read` the SKILL.md and
  // follow its instructions — they're not auto-executed.
  const { skills } = loadCorivoSkills({
    bundledSkillsDir: input.bundled_skills_dir,
  });
  prompt += formatSkillsForPrompt(skills);

  return prompt;
}
