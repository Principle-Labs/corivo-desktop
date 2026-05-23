// Native AgentTool: save_note (memory-system-spec §3.4.1, §10.3).
//
// Promotes a piece of declarative memory ("user told me to remember
// X") to the long-lived `notes` table. Step 1a:
//   * scope='global' + source='user_explicit' + status='active' rows
//     get rendered into the <persistent_memory> block every turn.
//   * source='agent_inferred' rows land as status='suggested' and
//     stay invisible to the prompt until the user confirms via the
//     Settings UI.
//
// Invocation rules belong in the system prompt — see Agent.md /
// default_agent.md. This tool just persists.

import { Type } from "typebox";
import type { AgentTool } from "@mariozechner/pi-agent-core";
import { rustRpc } from "../rpc.js";
import { isMockMode } from "../mock.js";

const Parameters = Type.Object({
  content: Type.String({
    description:
      "The natural-language fact / preference to remember. Should be self-contained — future turns won't have the surrounding conversation.",
  }),
  scope: Type.Optional(
    Type.Union(
      [Type.Literal("global"), Type.Literal("project"), Type.Literal("session")],
      {
        description:
          "How widely this note should apply. 'global' affects every thread (default); 'session' is bound to the current thread; 'project' is reserved for future use.",
      },
    ),
  ),
  source: Type.Optional(
    Type.Union(
      [Type.Literal("user_explicit"), Type.Literal("agent_inferred")],
      {
        description:
          "Who is asserting this. 'user_explicit' = the user said it directly (default). 'agent_inferred' = you noticed a pattern; lands as 'suggested' until the user confirms.",
      },
    ),
  ),
  source_message_id: Type.Optional(
    Type.String({
      description:
        "Optional message id this note was extracted from. If you have the originating user message id, pass it for auditability.",
    }),
  ),
  confidence: Type.Optional(
    Type.Number({
      description:
        "Optional 0..1 confidence. Defaults to 1.0 for user_explicit and 0.6 for agent_inferred.",
    }),
  ),
  reason: Type.Optional(
    Type.String({
      description:
        "Short note explaining WHY this is worth remembering long-term. Surfaces in the Settings UI to help the user review and prune.",
    }),
  ),
});

interface SaveNoteResult {
  id: string;
  scope: string;
  source_type: string;
  status: string;
  saved: boolean;
}

export const saveNoteTool: AgentTool<typeof Parameters> = {
  name: "save_note",
  label: "记住偏好",
  description:
    "Persist a user preference or fact to long-term memory. Use ONLY when the user explicitly says 记住 / 以后都 / 别再 / 我喜欢 / 我不喜欢, or when you spot a recurring preference worth promoting. Avoid for one-shot facts or current task state.",
  parameters: Parameters,
  execute: async (_toolCallId, params, signal) => {
    let result: SaveNoteResult;
    if (isMockMode()) {
      result = {
        id: "note_mock",
        scope: params.scope ?? "global",
        source_type: params.source ?? "user_explicit",
        status: (params.source ?? "user_explicit") === "agent_inferred"
          ? "suggested"
          : "active",
        saved: true,
      };
    } else {
      result = (await rustRpc(
        "save_note",
        params,
        signal,
      )) as SaveNoteResult;
    }

    const summary = result.status === "active"
      ? `已记住 (scope=${result.scope})`
      : `已加入候选 (status=${result.status}); 用户在设置页确认后才会生效`;

    return {
      content: [{ type: "text", text: summary }],
      details: result,
    };
  },
};
