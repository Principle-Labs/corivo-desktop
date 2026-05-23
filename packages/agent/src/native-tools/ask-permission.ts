// Native AgentTool: ask_permission (spec §6.2).
//
// Phase A: under CORIVO_AGENT_PHASE_A_MOCK=1 we auto-approve so loops can
// finish. Phase B routes through `rustRpc("ask_permission", ...)` which
// surfaces a UI prompt in the desktop main process.

import { Type } from "typebox";
import type { AgentTool } from "@mariozechner/pi-agent-core";
import { rustRpc } from "../rpc.js";
import { isMockMode } from "../mock.js";

const Parameters = Type.Object({
  action: Type.String({
    description: "Short identifier for the action being requested.",
  }),
  reason: Type.String({
    description: "Why the user should grant this permission.",
  }),
  details: Type.Optional(
    Type.Record(Type.String(), Type.Unknown(), {
      description:
        "Action-specific extra fields (e.g. command line, file path).",
    }),
  ),
});

interface AskPermissionResult {
  behavior: "allow" | "deny";
  message?: string;
}

export const askPermissionTool: AgentTool<typeof Parameters> = {
  name: "ask_permission",
  label: "请求用户授权",
  description:
    "Ask the user to approve a sensitive action. Returns behavior='allow'|'deny'.",
  parameters: Parameters,
  execute: async (_toolCallId, params, signal) => {
    let result: AskPermissionResult;
    if (isMockMode()) {
      result = { behavior: "allow", message: "(mock auto-approved)" };
    } else {
      result = (await rustRpc(
        "ask_permission",
        params,
        signal,
      )) as AskPermissionResult;
    }

    const approved = result.behavior === "allow";
    return {
      content: [
        {
          type: "text",
          text: approved
            ? `User approved: ${params.action}`
            : `User denied: ${params.action}${
                result.message ? ` (${result.message})` : ""
              }`,
        },
      ],
      details: result,
    };
  },
};
