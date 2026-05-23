// Native AgentTool: auto_persona_get_previous (memory-system-spec §4.3).
//
// Returns the previous version of `auto-persona.md` (or `""` if none).
// The persona distill task starts with this so it knows what stayed
// stable and what needs refreshing.

import { Type } from "typebox";
import type { AgentTool } from "@mariozechner/pi-agent-core";
import { rustRpc } from "../rpc.js";
import { isMockMode } from "../mock.js";

const Parameters = Type.Object({});

interface PreviousPersonaResult {
  exists: boolean;
  path: string;
  content: string;
}

export const autoPersonaGetPreviousTool: AgentTool<typeof Parameters> = {
  name: "auto_persona_get_previous",
  label: "读取上一版画像",
  description:
    "Return the contents of the previous auto-persona.md if it exists. Use this at the start of a persona distillation run to know what previously held, so you only refresh what's stale.",
  parameters: Parameters,
  execute: async (_toolCallId, _params, signal) => {
    let result: PreviousPersonaResult;
    if (isMockMode()) {
      result = { exists: false, path: "(mock)", content: "" };
    } else {
      result = (await rustRpc(
        "auto_persona_get_previous",
        {},
        signal,
      )) as PreviousPersonaResult;
    }
    const text = result.exists
      ? result.content
      : "(没有上一版 auto-persona.md — 这是首次蒸馏)";
    return {
      content: [{ type: "text", text }],
      details: result,
    };
  },
};
