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
          // The accepted values used to be unspecified — models guessed
          // `[\"frames\", \"chat_messages\"]` and hit a validator error
          // before they ever saw any hits. Listing the exact singular
          // literals here prevents the trip-on-first-call failure mode.
          'Restrict to specific memory layers. Accepted singular values: "frame" (screen history), "note" (long-term notes), "message" (past chat messages). Omit to search all three.',
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
    const text = result.hits.length === 0
      ? `没找到与 "${params.query}" 相关的记忆。`
      : renderHits(result);
    return {
      // `content` is what the LLM sees; `details` is metadata for the
      // UI layer that the model has no access to. Previously this
      // returned only the count summary ("找到 8 条记忆") in content
      // and put the actual `excerpt` / `content` of each hit into
      // `details`. The model dutifully ran the search, got back what
      // looked like an empty result, and answered with things like
      // "返回结果没有展开具体内容". Real incident 2026-05-25:
      // the user's "总结刚刚做了啥" workflow surfaced this — the
      // model did the right thing under the constraint, the
      // constraint was wrong.
      content: [{ type: "text", text }],
      details: result,
    };
  },
};

/// Per-hit text cap. Keeps token count bounded even when 20 frames
/// hit; each frame excerpt is already truncated upstream by the
/// recall service but defense-in-depth keeps payloads predictable.
const HIT_EXCERPT_CAP = 280;

function renderHits(result: MemorySearchResult): string {
  const head = `找到 ${result.hits.length} 条记忆 (notes ${result.diagnostics.note_hits} / messages ${result.diagnostics.message_hits} / frames ${result.diagnostics.frame_hits}):`;
  const lines: string[] = [head, ""];
  for (let i = 0; i < result.hits.length; i++) {
    const hit = result.hits[i];
    if (!hit) continue;
    const meta = renderHitHeader(hit, i + 1);
    const body = truncate(extractHitText(hit), HIT_EXCERPT_CAP);
    lines.push(meta);
    if (body) {
      // Indent the body two spaces so the model parses each hit as
      // one logical block rather than a free-floating paragraph.
      const indented = body
        .split("\n")
        .map((line) => `  ${line}`)
        .join("\n");
      lines.push(indented);
    }
    lines.push("");
  }
  return lines.join("\n").trimEnd();
}

/// One-line header per hit: position, kind, timestamp, app/context.
/// Keeps the model oriented when scanning a list of 8–20 results.
function renderHitHeader(hit: MemoryHit, position: number): string {
  const ts = formatTs(hit.ts);
  const parts: string[] = [`${position}. [${hit.kind}`];
  if (ts) parts.push(`· ${ts}`);
  if (hit.kind === "frame") {
    const app = typeof hit.app_name === "string" ? hit.app_name : null;
    const title = typeof hit.window_title === "string" ? hit.window_title : null;
    if (app) parts.push(`· ${app}`);
    if (title && title !== app) parts.push(`· ${title}`);
  } else if (hit.kind === "note") {
    const scope = typeof hit.scope === "string" ? hit.scope : null;
    if (scope) parts.push(`· ${scope}`);
  }
  return `${parts.join(" ")}]`;
}

/// Each kind stores its readable body under a different key.
function extractHitText(hit: MemoryHit): string {
  if (hit.kind === "frame") {
    return typeof hit.excerpt === "string" ? hit.excerpt : "";
  }
  // notes + messages both use `content`.
  return typeof hit.content === "string" ? hit.content : "";
}

/// Accept either an ISO string (frames/messages from Rust serialize
/// DbInstant that way) or a number (legacy). Best-effort — if we
/// can't parse, just drop the timestamp rather than embarrass the
/// model with garbage.
function formatTs(ts: unknown): string | null {
  if (typeof ts !== "string" && typeof ts !== "number") return null;
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return null;
  // Compact form: 2026-05-25 03:14 UTC. Seconds + milliseconds aren't
  // useful for the model and just eat tokens.
  const iso = d.toISOString();
  return `${iso.slice(0, 10)} ${iso.slice(11, 16)} UTC`;
}

function truncate(text: string, cap: number): string {
  const trimmed = text.trim();
  if (trimmed.length <= cap) return trimmed;
  return `${trimmed.slice(0, cap).trimEnd()}…`;
}
