// Native AgentTool: thread_search (memory-system-spec §12.4.1).
//
// Coarse "find me the past conversation about X" search over
// chat_threads.summary + summary_topics. Returns thread metadata + a
// short summary so the model can decide whether to drill in via
// `chat_thread_get`.

import { Type } from "typebox";
import type { AgentTool } from "@mariozechner/pi-agent-core";
import { rustRpc } from "../rpc.js";
import { isMockMode } from "../mock.js";

const Parameters = Type.Object({
  query: Type.String({
    description:
      "Free-form query — words, phrases, or short questions. Best for locating an old thread by topic ('useEffect bug', 'auth refactor').",
  }),
  limit: Type.Optional(
    Type.Number({
      description:
        "Maximum number of threads to return (default 5, max 20).",
      default: 5,
    }),
  ),
});

interface ThreadSearchHit {
  thread_id: string;
  title?: string | null;
  summary: string;
  summary_updated_at: string;
  created_at: string;
  score: number;
}

interface ThreadSearchResult {
  hits: ThreadSearchHit[];
}

export const threadSearchTool: AgentTool<typeof Parameters> = {
  name: "thread_search",
  label: "查找过往对话",
  description:
    "Search the user's past chat threads by topic. Returns thread ids + concise summaries; follow up with `chat_thread_get` to load the full message log of a hit.",
  parameters: Parameters,
  execute: async (_toolCallId, params, signal) => {
    let result: ThreadSearchResult;
    if (isMockMode()) {
      result = { hits: [] };
    } else {
      result = (await rustRpc(
        "thread_search",
        params,
        signal,
      )) as ThreadSearchResult;
    }
    const summary = result.hits.length === 0
      ? `没找到与 "${params.query}" 相关的历史对话。`
      : `找到 ${result.hits.length} 个相关 thread`;
    return {
      content: [{ type: "text", text: summary }],
      details: result,
    };
  },
};
