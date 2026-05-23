// Native AgentTool: memory_search (memory-system-spec §7.7).
//
// Cross-table memory recall: searches frames + notes + chat_messages
// in one call and returns scored hits the model can drill into.
// Supersedes recall_screen_history (which only queried frames).

import { Type } from "typebox";
import type { AgentTool } from "@mariozechner/pi-agent-core";
import { emit } from "../events.js";
import { rustRpc } from "../rpc.js";
import { isMockMode } from "../mock.js";

const Parameters = Type.Object({
  query: Type.String({
    description:
      "Free-form query. Best phrased like a search term, not a question (e.g. 'useEffect race', not 'how did we fix the useEffect race').",
  }),
  layers: Type.Optional(
    Type.Array(
      Type.Union([
        Type.Literal("frame"),
        Type.Literal("note"),
        Type.Literal("message"),
      ]),
      {
        description:
          "Restrict to specific memory layers. Omit to search all three.",
      },
    ),
  ),
});

interface MemoryHit {
  kind: "frame" | "note" | "message";
  id: string;
  score: number;
  // shape depends on kind; left loose intentionally
  [k: string]: unknown;
}

interface MemorySearchResult {
  hits: MemoryHit[];
  diagnostics: {
    frame_hits: number;
    note_hits: number;
    message_hits: number;
    elapsed_ms: number;
  };
}

export const memorySearchTool: AgentTool<typeof Parameters> = {
  name: "memory_search",
  label: "查记忆",
  description:
    "Search the user's long-term memory: notes (explicit preferences), frames (screen history), and chat_messages (past conversations) in one call. Use when you need information beyond what's already injected into your context.",
  parameters: Parameters,
  execute: async (_toolCallId, params, signal) => {
    let result: MemorySearchResult;
    if (isMockMode()) {
      result = {
        hits: [],
        diagnostics: {
          frame_hits: 0,
          note_hits: 0,
          message_hits: 0,
          elapsed_ms: 0,
        },
      };
    } else {
      result = (await rustRpc(
        "memory_search",
        params,
        signal,
      )) as MemorySearchResult;
    }
    // Mirror recall_screen_history's behavior: announce any cited
    // frames so the assistant turn's `cited_frame_ids` picks them up.
    const frameHits = result.hits
      .filter((h) => h.kind === "frame")
      .map((h) => String(h.id));
    if (frameHits.length > 0) {
      emit({
        type: "cited_frames",
        data: { frame_ids: frameHits },
      });
    }
    const summary = result.hits.length === 0
      ? `没找到与 "${params.query}" 相关的记忆。`
      : `找到 ${result.hits.length} 条记忆 (notes ${result.diagnostics.note_hits} / messages ${result.diagnostics.message_hits} / frames ${result.diagnostics.frame_hits})`;
    return {
      content: [{ type: "text", text: summary }],
      details: result,
    };
  },
};
