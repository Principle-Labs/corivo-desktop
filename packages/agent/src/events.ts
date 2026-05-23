// Corivo wire envelope (spec §5.2). All sidecar→Rust events go through
// `emit()` which writes one NDJSON line to stdout.

import type { Usage } from "@mariozechner/pi-ai";

export const SCHEMA_VERSION = 1;

export type FinishReason =
  | "EndTurn"
  | "ToolUse"
  | "Refusal"
  | "Error"
  | "Cancelled";

// Discriminated union of every event type carried by the envelope.
export type CorivoEvent =
  | { type: "agent_start"; data: { session_id: string; model: string } }
  | { type: "turn_start"; data: { turn_index: number } }
  | { type: "text_delta"; data: { delta: string } }
  | { type: "thinking_delta"; data: { delta: string } }
  | {
      type: "tool_call_start";
      data: { tool_call_id: string; name: string; arguments: string };
    }
  | {
      type: "tool_call_end";
      data: { tool_call_id: string; name: string; arguments: string };
    }
  | {
      type: "tool_execution_start";
      data: { tool_call_id: string; name: string; args: unknown };
    }
  | {
      type: "tool_execution_update";
      data: { tool_call_id: string; partial_result: unknown };
    }
  | {
      type: "tool_execution_end";
      data: {
        tool_call_id: string;
        result: unknown;
        is_error: boolean;
      };
    }
  | { type: "cited_frames"; data: { frame_ids: string[] } }
  | { type: "compaction_start"; data: { reason: "auto" | "manual" } }
  | {
      type: "compaction_end";
      data: { removed_tokens: number; kept_tokens: number };
    }
  | {
      type: "turn_end";
      data: {
        assistant_message: unknown;
        tool_results: unknown[];
        usage?: Usage;
      };
    }
  | {
      type: "agent_end";
      data: { finish_reason: FinishReason; usage_total?: Usage };
    }
  | {
      type: "error";
      data: { code: string; message: string; recoverable: boolean };
    }
  | {
      type: "api_retry";
      data: {
        attempt: number;
        max_attempts: number;
        delay_ms: number;
        status?: number;
      };
    };

export interface CorivoEnvelope {
  v: number;
  ts: number;
  type: CorivoEvent["type"];
  data: CorivoEvent["data"];
}

/**
 * Serialize an event into a single NDJSON line on stdout.
 *
 * Bun-compiled binaries inherit Node's `process.stdout`. We write synchronously
 * to ensure ordering and avoid interleaving across awaits.
 */
export function emit(event: CorivoEvent): void {
  const envelope: CorivoEnvelope = {
    v: SCHEMA_VERSION,
    ts: Date.now(),
    type: event.type,
    data: event.data,
  };
  const line = JSON.stringify(envelope);
  process.stdout.write(`${line}\n`);
}
