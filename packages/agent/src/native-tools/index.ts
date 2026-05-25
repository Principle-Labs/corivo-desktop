import { existsSync } from "node:fs";
import os from "node:os";
import type { AgentTool } from "@mariozechner/pi-agent-core";
import {
  createBashTool,
  createEditTool,
  createFindTool,
  createGrepTool,
  createLsTool,
  createReadTool,
  createWriteTool,
} from "@mariozechner/pi-coding-agent";
import { recallScreenHistoryTool } from "./recall-screen-history.js";
import { askPermissionTool } from "./ask-permission.js";
import { saveNoteTool } from "./save-note.js";
import { threadSearchTool } from "./thread-search.js";
import { chatThreadGetTool } from "./chat-thread-get.js";
import { noteListTool } from "./note-list.js";
import { memorySearchTool } from "./memory-search.js";
import { autoPersonaGetPreviousTool } from "./auto-persona-get-previous.js";
import {
  cancelScheduledTaskTool,
  listScheduledTasksTool,
  scheduleTaskTool,
  updateScheduledTaskTool,
} from "./schedule-task.js";

/**
 * Working directory for pi-coding-agent's bash / read / write / edit /
 * grep / find / ls tools. The sidecar is launched by Tauri so
 * `process.cwd()` points at the app bundle directory which is useless;
 * default to the user's home so the model can `cd` around to wherever
 * it actually needs to operate.
 *
 * TODO(spec): plumb a per-turn workspace cwd through `SidecarInput`
 * (e.g. derived from focus_context or an explicit Settings field)
 * once we have product UX for "the agent's working directory".
 */
const TOOL_CWD = os.homedir();

/**
 * Resolve the shell binary pi-coding-agent's `bash` tool should spawn.
 *
 * On macOS / Linux: return undefined and let pi pick the system bash
 * (/bin/bash → `bash` on PATH → fall back to sh).
 *
 * On Windows: pi's default shell-resolution chain looks for Git Bash and
 * then `bash.exe` on PATH, and throws when neither is present. Most
 * Windows users don't have Git installed, so the bash tool errors out
 * on the first call. Instead, pick PowerShell:
 *
 *   1. PowerShell 7 (`pwsh.exe`) under the default per-user / system
 *      install paths — preferred because it's modern, cross-platform,
 *      and what the system prompt tells the model to write for.
 *   2. Windows PowerShell 5 (`powershell.exe`) — ships on every supported
 *      Windows install since Windows 7. Reasonable fallback.
 *
 * pi-coding-agent will pass `-c <command>` to whatever shell we hand it,
 * which both pwsh and Windows PowerShell accept (it's an alias for
 * `-Command`). The system prompt in system-prompt.ts tells the model
 * that the host shell is PowerShell on Windows so it writes
 * cmdlet-flavored commands instead of POSIX-only constructs.
 */
function resolveHostShellPath(): string | undefined {
  if (process.platform !== "win32") return undefined;

  const candidates: string[] = [];
  if (process.env.ProgramFiles) {
    candidates.push(`${process.env.ProgramFiles}\\PowerShell\\7\\pwsh.exe`);
  }
  if (process.env["ProgramFiles(x86)"]) {
    candidates.push(`${process.env["ProgramFiles(x86)"]}\\PowerShell\\7\\pwsh.exe`);
  }
  // Per-user winget / scoop install location.
  if (process.env.LOCALAPPDATA) {
    candidates.push(
      `${process.env.LOCALAPPDATA}\\Microsoft\\WinGet\\Links\\pwsh.exe`,
    );
  }
  // Windows PowerShell 5 ships everywhere — universal fallback.
  if (process.env.SystemRoot) {
    candidates.push(
      `${process.env.SystemRoot}\\System32\\WindowsPowerShell\\v1.0\\powershell.exe`,
    );
  } else {
    candidates.push(
      "C\\:Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",
    );
  }

  for (const c of candidates) {
    if (existsSync(c)) return c;
  }
  // None of the candidates exist — return undefined and let pi's bash
  // chain take over (it'll throw its own error, which is still clearer
  // than a silent fallback to a broken shell).
  return undefined;
}

const HOST_SHELL_PATH = resolveHostShellPath();

/**
 * Build the full pi-coding-agent native tool set, keyed by tool name
 * matching what we hardcode on the Rust side (corivo.rs `tools.native`).
 *
 * pi's helper `createCodingTools` is misleadingly named — it returns
 * only the 4-tool *editing* subset (read/bash/edit/write), and
 * `createReadOnlyTools` returns the 4-tool *read-only* subset
 * (read/grep/find/ls). Neither alone is what we want, and using
 * `createAllTools` would give us the same Record with one extra hop.
 * Calling each factory individually is clearest about intent.
 */
function buildCodingToolMap(): Record<string, AgentTool<any>> {
  return {
    read: createReadTool(TOOL_CWD),
    // shellPath = pwsh / powershell.exe on Windows; undefined elsewhere
    // (pi's default `/bin/bash` chain). See resolveHostShellPath above
    // and the matching note in system-prompt.ts.
    bash: createBashTool(TOOL_CWD, HOST_SHELL_PATH ? { shellPath: HOST_SHELL_PATH } : undefined),
    edit: createEditTool(TOOL_CWD),
    write: createWriteTool(TOOL_CWD),
    grep: createGrepTool(TOOL_CWD),
    find: createFindTool(TOOL_CWD),
    ls: createLsTool(TOOL_CWD),
  };
}

const REGISTRY: Record<string, AgentTool<any>> = {
  recall_screen_history: recallScreenHistoryTool,
  memory_search: memorySearchTool,
  ask_permission: askPermissionTool,
  save_note: saveNoteTool,
  thread_search: threadSearchTool,
  chat_thread_get: chatThreadGetTool,
  note_list: noteListTool,
  auto_persona_get_previous: autoPersonaGetPreviousTool,
  // v1431 — scheduled-workflow lifecycle (services::scheduled_workflows
  // on the Rust side). Each tool round-trips through `rustRpc` →
  // `schedule_task_handler` etc. Storage is shared with the /workflows
  // page so the user can see + edit anything the agent creates.
  schedule_task: scheduleTaskTool,
  list_scheduled_tasks: listScheduledTasksTool,
  cancel_scheduled_task: cancelScheduledTaskTool,
  update_scheduled_task: updateScheduledTaskTool,
  ...buildCodingToolMap(),
};

export function resolveNativeTools(names: string[]): AgentTool<any>[] {
  const out: AgentTool<any>[] = [];
  for (const name of names) {
    const tool = REGISTRY[name];
    if (!tool) {
      throw new Error(`unknown native tool: ${name}`);
    }
    out.push(tool);
  }
  return out;
}

export const KNOWN_NATIVE_TOOLS = Object.keys(REGISTRY);
