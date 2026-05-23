// Native AgentTool: recall_screen_history (spec §6.2).
//
// Phase A: when CORIVO_AGENT_PHASE_A_MOCK=1 we short-circuit the UDS round
// trip and return a static fake payload so the sidecar can be exercised
// without a Rust counterpart. Phase B replaces the mock branch with a real
// `rustRpc("recall_screen_history", ...)` call.

import { Type } from "typebox";
import type { AgentTool } from "@mariozechner/pi-agent-core";
import { emit } from "../events.js";
import { rustRpc } from "../rpc.js";
import { isMockMode } from "../mock.js";

const Parameters = Type.Object({
  query: Type.String({
    description: "Free-form query to search the user's screen history.",
  }),
  limit: Type.Optional(
    Type.Number({
      description: "Maximum number of frames to return (default 10).",
      default: 10,
    }),
  ),
});

interface RecallResult {
  summary: string;
  frames: Array<{ id: string; ts: number; text?: string }>;
}

export const recallScreenHistoryTool: AgentTool<typeof Parameters> = {
  name: "recall_screen_history",
  label: "查询屏幕历史",
  description:
    "Search the user's screen history for relevant frames. Returns a summary plus a list of frame ids that the assistant should cite.",
  parameters: Parameters,
  execute: async (_toolCallId, params, signal) => {
    let result: RecallResult;
    if (isMockMode()) {
      result = {
        summary: `(mock) found 0 frames for query: ${params.query}`,
        frames: [],
      };
    } else {
      result = (await rustRpc(
        "recall_screen_history",
        params,
        signal,
      )) as RecallResult;
    }

    if (result.frames.length > 0) {
      emit({
        type: "cited_frames",
        data: { frame_ids: result.frames.map((f) => f.id) },
      });
    }

    return {
      content: [{ type: "text", text: result.summary }],
      details: { frames: result.frames },
    };
  },
};
