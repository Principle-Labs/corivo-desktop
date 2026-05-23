// Connector framework — agent-side types.
//
// `ConnectorCtx` itself now lives in `@corivo/shared-types` so connector
// packages can depend on it without pulling in `@corivo/agent` and
// creating a dep cycle (agent depends on each connector via the
// `BUILTIN` map). The agent re-exports it from here for ergonomics +
// in case we ever want to extend it in an agent-private way.
//
// The other two types in this file (`CreateToolsFn`,
// `ConnectorRefreshBridge`) are agent-internal — connector authors
// don't need them — so they stay here.

import type { AgentTool } from "@mariozechner/pi-agent-core";
export type { ConnectorCtx } from "@corivo/shared-types";
import type { ConnectorCtx } from "@corivo/shared-types";

/**
 * Factory contract every connector package's `index.ts` must export.
 * Convention: `export function createTools(ctx: ConnectorCtx): AgentTool[]`
 * — a named export, not default, so cross-package imports stay explicit.
 */
export type CreateToolsFn = (ctx: ConnectorCtx) => AgentTool<any>[];

/**
 * Host-side capability that `ctx.fetch` calls when an access_token is
 * approaching expiry or has just been rejected with 401. The Phase 1
 * agent passes a rejecting stub (mid-turn refresh deferred to Phase 2,
 * see `agent-runner.ts`); the connector code path is unaffected
 * because `ctx.fetch` already handles a rejecting bridge.
 */
export interface ConnectorRefreshBridge {
  /** Ask Rust for a fresh access_token. Returns the new token + expiry
   *  on success, or rejects with the upstream error message. */
  refresh(connectorId: string): Promise<{
    accessToken: string;
    expiresAt: string | null;
  }>;
}
