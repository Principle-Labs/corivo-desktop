// Native AgentTool: chat_thread_get (memory-system-spec §10.3).
//
// Read-only loader for an entire chat thread. Both user-facing turns
// and background agent tasks (session learner) use this to inspect
// past message logs.

import { Type } from "typebox";
import type { AgentTool } from "@mariozechner/pi-agent-core";
import { rustRpc } from "../rpc.js";
import { isMockMode } from "../mock.js";

const Parameters = Type.Object({
  thread_id: Type.String({
    description: "The thread id (ULID) to fetch. Must be a kind='user' thread.",
  }),
  after_message_id: Type.Optional(
    Type.String({
      description:
        "If supplied, only messages created strictly AFTER this id are returned. Useful for incremental learning over an already-summarized thread.",
    }),
  ),
});

interface MessageRow {
  id: string;
  role: "user" | "assistant" | "system";
  content_text: string;
  created_at: string;
}

interface ThreadGetResult {
  thread_id: string;
  title?: string | null;
  messages: MessageRow[];
  summary?: string | null;
  summary_updated_at?: string | null;
}

export const chatThreadGetTool: AgentTool<typeof Parameters> = {
  name: "chat_thread_get",
  label: "读取对话历史",
  description:
    "Fetch the complete message log of a chat thread. Use to pull historical context (e.g. after thread_search) or to learn from a thread that has gone idle.",
  parameters: Parameters,
  execute: async (_toolCallId, params, signal) => {
    let result: ThreadGetResult;
    if (isMockMode()) {
      result = { thread_id: params.thread_id, messages: [] };
    } else {
      result = (await rustRpc(
        "chat_thread_get",
        params,
        signal,
      )) as ThreadGetResult;
    }
    const lines = result.messages.map((m) => {
      const trimmed = m.content_text.length > 240
        ? m.content_text.slice(0, 240) + "…"
        : m.content_text;
      return `[${m.role} ${m.created_at}] ${trimmed}`;
    });
    const text = lines.length === 0
      ? `(thread "${params.thread_id}" 没有可读消息)`
      : lines.join("\n");
    return {
      content: [{ type: "text", text }],
      details: result,
    };
  },
};
