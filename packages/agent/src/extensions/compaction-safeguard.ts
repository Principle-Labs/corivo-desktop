// Compaction safeguard (spec §8.3 fallback) — the ONLY in-process,
// per-LLM-round defense.
//
// Cross-turn smart summarization is now driven from
// `agent-runner.ts::runAgent` AFTER each turn ends:它调
// `@mariozechner/pi-coding-agent` 的 `prepareCompaction` + `compact`,
// 把结果通过 `SessionManager.appendCompaction` 持久化进 jsonl。下次
// sidecar boot 时 `buildSessionContext()` 自动用 summary 替换 head。
//
// 这条 safeguard 留给一种罕见场景:**单 turn 内** tool result 直接把
// messages 顶到 contextWindow × 0.95 以上,LLM summary 来不及救。这
// 时没有别的选择,只能丢掉最老的非系统消息,直到回落。纯内存,不写
// jsonl —— 它救的是「这个 LLM round」,不应该改写持久历史。

import type { AgentMessage } from "@mariozechner/pi-agent-core";

import { emit } from "../events.js";
import type { BuiltModel } from "../auth.js";

/** Hard truncate when estimate > contextWindow × this. */
export const SAFEGUARD_HARD_RATIO = 0.95;

/** Rough token estimate per message: `chars / 4 + per-message overhead`.
 *  Conservative — used only for the safeguard trigger; an off-by-15%
 *  estimate is fine for "are we above 0.95 of the window?". */
const PER_MESSAGE_OVERHEAD = 4;

export interface SafeguardOptions {
  main: BuiltModel;
}

/** Returns the messages array trimmed to fit under the hard ceiling.
 *  If already under, returns the original array unchanged. */
export function applyCompactionSafeguard(
  messages: AgentMessage[],
  opts: SafeguardOptions,
): AgentMessage[] {
  const contextWindow = opts.main.model.contextWindow ?? 0;
  if (contextWindow <= 0) return messages;
  const hardCeiling = Math.floor(contextWindow * SAFEGUARD_HARD_RATIO);

  let estimate = estimateTokens(messages);
  if (estimate <= hardCeiling) return messages;

  // Drop oldest non-system messages until under. We mutate a copy so the
  // caller's array stays intact (pi-agent-core's `transformContext`
  // contract expects either the same array or a new one).
  const trimmed: AgentMessage[] = [...messages];
  let dropped = 0;
  while (trimmed.length > 1 && estimate > hardCeiling) {
    const idx = oldestDroppableIndex(trimmed);
    if (idx < 0) break;
    trimmed.splice(idx, 1);
    dropped += 1;
    estimate = estimateTokens(trimmed);
  }

  if (dropped > 0) {
    emit({
      type: "error",
      data: {
        code: "compaction_overflow_truncated",
        message: `compaction safeguard hard-truncated ${dropped} message(s) — context still exceeded ${hardCeiling} tokens within a single turn`,
        recoverable: true,
      },
    });
  }
  return trimmed;
}

/** Token estimator — exported so tests + the cross-turn driver can share
 *  the same ruler. */
export function estimateTokens(messages: ReadonlyArray<AgentMessage>): number {
  let total = 0;
  for (const m of messages) {
    total += PER_MESSAGE_OVERHEAD;
    total += Math.ceil(messageTextLength(m) / 4);
  }
  return total;
}

function messageTextLength(m: AgentMessage): number {
  const content = (m as { content?: unknown; summary?: unknown }).content;
  if (typeof content === "string") return content.length;
  if (Array.isArray(content)) {
    let n = 0;
    for (const part of content) {
      if (!part || typeof part !== "object") continue;
      const p = part as { type?: string; text?: string; thinking?: string };
      if (p.type === "text" && typeof p.text === "string") n += p.text.length;
      else if (p.type === "thinking" && typeof p.thinking === "string")
        n += p.thinking.length;
    }
    return n;
  }
  // Compaction/branch summary AgentMessages carry the text under `summary`.
  const summary = (m as { summary?: unknown }).summary;
  if (typeof summary === "string") return summary.length;
  return 0;
}

function oldestDroppableIndex(messages: ReadonlyArray<AgentMessage>): number {
  // Skip the very last message (the live user/tool-result turn) and any
  // existing compactionSummary entries — undoing them would burn the
  // smart summary the cross-turn driver paid an LLM call to produce.
  for (let i = 0; i < messages.length - 1; i++) {
    const role = (messages[i] as { role?: string }).role;
    if (role === "compactionSummary" || role === "branchSummary") continue;
    return i;
  }
  return -1;
}
