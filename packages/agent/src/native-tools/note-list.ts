// Native AgentTool: note_list (memory-system-spec §10.3).
//
// Read-only enumerator over the `notes` table. Both user-facing turns
// (rare — the persistent block already injects globals) and the
// session learner (to avoid duplicate suggestions) use this.

import { Type } from "typebox";
import type { AgentTool } from "@mariozechner/pi-agent-core";
import { rustRpc } from "../rpc.js";
import { isMockMode } from "../mock.js";

const Parameters = Type.Object({
  scope: Type.Optional(
    Type.Union(
      [Type.Literal("global"), Type.Literal("project"), Type.Literal("session")],
      { description: "Filter by scope. Omit to return all scopes." },
    ),
  ),
  status: Type.Optional(
    Type.Union(
      [
        Type.Literal("active"),
        Type.Literal("suggested"),
        Type.Literal("superseded"),
        Type.Literal("contradicted"),
        Type.Literal("archived"),
      ],
      { description: "Filter by status. Omit to return all statuses." },
    ),
  ),
  source: Type.Optional(
    Type.Union(
      [Type.Literal("user_explicit"), Type.Literal("agent_inferred")],
      { description: "Filter by note source." },
    ),
  ),
  limit: Type.Optional(
    Type.Number({ description: "Maximum number of notes to return (default unbounded)." }),
  ),
});

interface NoteRow {
  id: string;
  content: string;
  scope: string;
  source_type: string;
  status: string;
  confidence: number;
  created_at: string;
}

interface NoteListResult {
  notes: NoteRow[];
}

export const noteListTool: AgentTool<typeof Parameters> = {
  name: "note_list",
  label: "查询已记忆条目",
  description:
    "List notes from the long-term memory store. Use with scope='global' + status='active' to see what Corivo will already remember every turn — useful to avoid suggesting duplicates.",
  parameters: Parameters,
  execute: async (_toolCallId, params, signal) => {
    let result: NoteListResult;
    if (isMockMode()) {
      result = { notes: [] };
    } else {
      result = (await rustRpc(
        "note_list",
        params,
        signal,
      )) as NoteListResult;
    }
    const summary = result.notes.length === 0
      ? "(没有匹配的 note)"
      : `共 ${result.notes.length} 条`;
    return {
      content: [{ type: "text", text: summary }],
      details: result,
    };
  },
};
