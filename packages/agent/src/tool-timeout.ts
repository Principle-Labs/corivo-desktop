// Per-tool execution deadline.
//
// Contract: every tool call must produce a result (success or error) within
// a bounded time. A hung tool — e.g. `grep` blocked on a pathological Windows
// path, or an MCP server that wedges mid-call — must not be able to freeze
// a turn indefinitely. When the deadline fires we abort the in-flight call
// and throw an actionable error; pi-agent-core wraps that as `isError: true`
// toolResult and the next LLM round can pick a narrower strategy.

import type { AgentTool } from "@mariozechner/pi-agent-core";
import { log } from "./log.js";

/**
 * Default per-tool execution deadline.
 *
 * Calibrated against observed legitimate-but-slow runs (`grep` / `find` over
 * the entire user home took 15–19s on Windows) with ~1.5× headroom. Tools
 * that legitimately need longer should either accept a `timeout` parameter
 * and opt out below, or be split into smaller calls by the model.
 */
const DEFAULT_TOOL_TIMEOUT_MS = 30_000;

/**
 * Tools that opt out of the global wrapper.
 *
 * - `bash` accepts its own `timeout` parameter and self-enforces; a fixed
 *   30s cap here would silently shorten user-requested timeouts.
 * - `ask_permission` is user-driven and indefinite by design — it blocks
 *   until the human responds (or the user cancels).
 *
 * Default behavior: everything else is bounded. Add a name here only with
 * a written reason.
 */
const TIMEOUT_EXEMPT_TOOLS: ReadonlySet<string> = new Set([
  "bash",
  "ask_permission",
]);

export function wrapToolsWithTimeout<T extends AgentTool<any, any>>(
  tools: T[],
  timeoutMs: number = DEFAULT_TOOL_TIMEOUT_MS,
): T[] {
  return tools.map((tool) => {
    if (TIMEOUT_EXEMPT_TOOLS.has(tool.name)) return tool;
    return { ...tool, execute: withTimeout(tool, timeoutMs) } as T;
  });
}

function withTimeout<T extends AgentTool<any, any>>(
  tool: T,
  timeoutMs: number,
): T["execute"] {
  const original = tool.execute.bind(tool);
  return (toolCallId, params, userSignal, onUpdate) => {
    const timeoutController = new AbortController();
    const composed = composeSignals([userSignal, timeoutController.signal]);

    return new Promise((resolve, reject) => {
      let settled = false;

      const timer = setTimeout(() => {
        if (settled) return;
        settled = true;
        log.warn("agent.tool_timeout", {
          tool: tool.name,
          tool_call_id: toolCallId,
          timeout_ms: timeoutMs,
        });
        // Best-effort tell the in-flight tool to clean up. Whether it
        // actually unwinds depends on the tool implementation honoring
        // the signal — we don't wait for that, the wrapper rejects now.
        timeoutController.abort();
        reject(
          new Error(
            `tool \`${tool.name}\` timed out after ${Math.round(timeoutMs / 1000)}s. ` +
              `Retry with a narrower scope (more specific path / pattern) or split into smaller calls.`,
          ),
        );
      }, timeoutMs);

      original(toolCallId, params, composed, onUpdate).then(
        (result) => {
          if (settled) return;
          settled = true;
          clearTimeout(timer);
          resolve(result);
        },
        (err) => {
          if (settled) return;
          settled = true;
          clearTimeout(timer);
          reject(err);
        },
      );
    });
  };
}

/**
 * Compose multiple AbortSignals: the returned signal aborts as soon as any
 * input signal aborts. Used to fan in (a) the user-driven cancel signal from
 * pi-agent-core and (b) our internal timeout signal so the inner tool sees
 * a single signal and can honor whichever fires first.
 */
function composeSignals(
  signals: Array<AbortSignal | undefined>,
): AbortSignal {
  const real = signals.filter((s): s is AbortSignal => !!s);
  if (real.length === 0) return new AbortController().signal;
  if (real.length === 1) return real[0];

  const controller = new AbortController();
  for (const s of real) {
    if (s.aborted) {
      controller.abort(s.reason);
      return controller.signal;
    }
    s.addEventListener("abort", () => controller.abort(s.reason), {
      once: true,
    });
  }
  return controller.signal;
}
