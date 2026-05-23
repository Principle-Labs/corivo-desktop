// Cross-turn smart compaction (spec §8.3 primary path).
//
// Driven from `agent-runner.ts` AFTER `Agent.waitForIdle()` returns —
// i.e. once per sidecar invocation, between the turn that just finished
// and whatever the next sidecar boot will do. Writes a single
// `compaction` entry to the SessionManager jsonl;
// `buildSessionContext()` on the next boot expands that entry into a
// `compactionSummary` AgentMessage that pi-coding-agent's `convertToLlm`
// turns into a single `user` message with the standard summary
// prefix/suffix. Net effect: the LLM only ever sees the summary plus
// recent turns, but the raw history is preserved on disk for
// debugging/audit.
//
// Why we don't reuse pi-coding-agent's `prepareCompaction`/`compact`:
// neither is re-exported from the package's main entry — the
// package.json `exports` map only exposes `.` and `./hooks`, and the
// main `index.d.ts` deliberately omits them. We re-implement the
// minimum needed (cut-point + summarize + appendCompaction) so we don't
// depend on internal paths that could break across pi-coding-agent
// upgrades.

import { complete } from "@mariozechner/pi-ai";
import type {
  Context,
  Message,
  Model,
  UserMessage,
} from "@mariozechner/pi-ai";
import type {
  AgentMessage,
  AgentTool,
} from "@mariozechner/pi-agent-core";
import type { SessionManager } from "@mariozechner/pi-coding-agent";

import { emit } from "./events.js";
import { log } from "./log.js";
import type { BuiltModel } from "./auth.js";
import { estimateTokens } from "./extensions/compaction-safeguard.js";

/** Trigger compaction when estimated tokens >= contextWindow * this. */
export const COMPACTION_TRIGGER_RATIO = 0.75;

/** Keep at least this many user-introduced turns intact at the tail.
 *  A "turn" here = one user message + the trailing assistant/tool-result
 *  messages until the next user message. spec §8.3 mandates ≥ 4. */
export const MIN_KEEP_TURNS = 4;

/** UA stamped on the compaction LLM call — same Cloudflare-WAF concern
 *  as the main stream call (see CORIVO_AGENT_USER_AGENT in agent-runner.ts).
 *  Duplicated as a string here to avoid a circular import. */
const COMPACTION_UA_HEADERS = {
  "user-agent": "corivo-agent/0.0.1 (+https://corivo.ai)",
};

const COMPACTION_SYSTEM_PROMPT =
  "你是一个上下文压缩助手。把下面这段较早的对话历史浓缩成一段 200 字以内的中文摘要,保留:" +
  "(1) 用户的关键意图/目标;(2) 已经确定的事实;(3) agent 调用过的关键工具与重要返回结果。" +
  "不要添加任何新信息。直接返回摘要文本,不要 'Summary:' 之类的前缀。";

function elapsedMs(startedAt: number): number {
  return Math.max(0, Date.now() - startedAt);
}

export interface CrossTurnCompactionOptions {
  sm: SessionManager;
  /** Snapshot of the agent's resolved transcript (post any prior compactions
   *  applied at boot via `buildSessionContext()`). Used only to estimate the
   *  trigger threshold — the cut decision walks `sm.getEntries()`. */
  currentMessages: AgentMessage[];
  /** Main model — drives the contextWindow used in the trigger threshold. */
  main: BuiltModel;
  /** Cheap model for the summary call. */
  compaction: BuiltModel;
}

/**
 * One compaction pass. No-op when:
 *   - main contextWindow is unknown (=0)
 *   - tokens estimate < contextWindow × COMPACTION_TRIGGER_RATIO
 *   - sm has fewer than MIN_KEEP_TURNS user messages **since the last
 *     compaction entry** (nothing to compact)
 *
 * Failures (LLM call, persistence) are logged + emitted as recoverable
 * errors, but do not propagate — the user's turn already succeeded;
 * losing a compaction only costs a bit of input on the next boot.
 */
export async function runCrossTurnCompaction(
  opts: CrossTurnCompactionOptions,
): Promise<void> {
  const startedAt = Date.now();
  const { sm, currentMessages, main, compaction } = opts;

  const contextWindow = main.model.contextWindow ?? 0;
  if (contextWindow <= 0) {
    log.info("agent.compaction.skip_no_context_window", {
      elapsed_ms: elapsedMs(startedAt),
    });
    return;
  }
  const threshold = Math.floor(contextWindow * COMPACTION_TRIGGER_RATIO);

  const tokensCurrent = estimateTokens(currentMessages);
  log.info("agent.compaction.check", {
    tokens: tokensCurrent,
    threshold,
    window: contextWindow,
    elapsed_ms: elapsedMs(startedAt),
  });
  if (tokensCurrent < threshold) {
    log.info("agent.compaction.skip_under_threshold", {
      tokens: tokensCurrent,
      threshold,
      window: contextWindow,
      elapsed_ms: elapsedMs(startedAt),
    });
    return;
  }

  // Walk sm entries forward, building a parallel (entryId, message) list
  // for message entries. Reset on `compaction` entries so the keep
  // boundary only considers messages added SINCE the last compaction —
  // otherwise we'd keep paying summarization cost on already-summarized
  // history.
  const entries = sm.getEntries();
  let messageEntries: Array<{ id: string; message: AgentMessage }> = [];
  for (const e of entries) {
    if (e.type === "message") {
      messageEntries.push({
        id: e.id,
        message: (e as { message: AgentMessage }).message,
      });
    } else if (e.type === "compaction") {
      messageEntries = [];
    }
  }
  if (messageEntries.length === 0) {
    log.info("agent.compaction.skip_no_messages", {
      entries: entries.length,
      elapsed_ms: elapsedMs(startedAt),
    });
    return;
  }

  // Walk back collecting user-message turns until we've kept MIN_KEEP_TURNS.
  // splitIdx = index in messageEntries where the kept tail starts; messages
  // BEFORE this index get summarized.
  let userTurnsSeen = 0;
  let splitIdx = -1;
  for (let i = messageEntries.length - 1; i >= 0; i--) {
    const role = (messageEntries[i].message as { role?: string }).role;
    if (role === "user") {
      userTurnsSeen += 1;
      if (userTurnsSeen >= MIN_KEEP_TURNS) {
        splitIdx = i;
        break;
      }
    }
  }
  if (splitIdx <= 0) {
    log.info("agent.compaction.skip_not_enough_turns", {
      message_entries: messageEntries.length,
      user_turns_seen: userTurnsSeen,
      min_keep: MIN_KEEP_TURNS,
      elapsed_ms: elapsedMs(startedAt),
    });
    return;
  }

  const firstKeptEntryId = messageEntries[splitIdx].id;
  const headMessages = messageEntries.slice(0, splitIdx).map((m) => m.message);
  if (headMessages.length === 0) {
    log.info("agent.compaction.skip_empty_head", {
      elapsed_ms: elapsedMs(startedAt),
    });
    return;
  }
  const tokensBefore = estimateTokens(headMessages);

  emit({ type: "compaction_start", data: { reason: "auto" } });

  let summary: string;
  try {
    summary = await summarizeWithModel(compaction, headMessages);
  } catch (err) {
    const detail = (err as Error).message ?? String(err);
    log.error("agent.compaction.summary_failed", { message: detail });
    emit({
      type: "error",
      data: {
        code: "compaction_failed",
        message: `cross-turn compaction summary failed: ${detail}`,
        recoverable: true,
      },
    });
    return;
  }

  try {
    sm.appendCompaction(
      summary,
      firstKeptEntryId,
      tokensBefore,
      undefined,
      true /* fromHook = true */,
    );
  } catch (err) {
    const detail = (err as Error).message ?? String(err);
    log.error("agent.compaction.append_failed", { message: detail });
    emit({
      type: "error",
      data: {
        code: "compaction_failed",
        message: `cross-turn compaction persist failed: ${detail}`,
        recoverable: true,
      },
    });
    return;
  }

  emit({
    type: "compaction_end",
    data: {
      removed_tokens: tokensBefore,
      kept_tokens: Math.max(0, tokensCurrent - tokensBefore),
    },
  });
  log.info("agent.compaction.done", {
    tokens_before: tokensBefore,
    summary_chars: summary.length,
    first_kept_entry_id: firstKeptEntryId,
    head_messages: headMessages.length,
    total_ms: elapsedMs(startedAt),
  });
}

async function summarizeWithModel(
  built: BuiltModel,
  messages: AgentMessage[],
): Promise<string> {
  // Project AgentMessage[] → Message[]. The compaction model gets a
  // self-contained one-shot request, so we can drop any custom roles
  // (compactionSummary / branchSummary / bashExecution / custom) — those
  // shouldn't reach this function in practice (we walk POST-compaction
  // entries only, and message entries hold raw pi-ai Messages), but the
  // filter is defensive against future custom message types.
  const projected = messages.filter(isLlmMessage) as Message[];
  if (projected.length === 0) {
    throw new Error("no messages eligible for summarization");
  }

  const ctx: Context = {
    systemPrompt: COMPACTION_SYSTEM_PROMPT,
    messages: projected,
    tools: [] as AgentTool<any>[],
  };
  const apiKey = built.streamOptions.apiKey;
  if (!apiKey) {
    throw new Error("compaction model has no apiKey configured");
  }
  const result = await complete(
    built.model as
      | Model<"anthropic-messages">
      | Model<"openai-completions">
      | Model<"openai-responses">,
    ctx,
    {
      apiKey,
      headers: COMPACTION_UA_HEADERS,
    },
  );

  if (result.stopReason === "error") {
    throw new Error(
      `compaction model returned stopReason=error: ${
        result.errorMessage ?? "(no errorMessage)"
      }`,
    );
  }
  const out = result.content
    .filter((c): c is { type: "text"; text: string } => c.type === "text")
    .map((c) => c.text)
    .join("\n")
    .trim();
  if (out.length === 0) {
    throw new Error("compaction model returned no text content");
  }
  return out;
}

function isLlmMessage(m: AgentMessage): boolean {
  const role = (m as Partial<UserMessage>).role;
  return role === "user" || role === "assistant" || role === "toolResult";
}
