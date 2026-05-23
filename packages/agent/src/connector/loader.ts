// Connector loader — translates `SidecarInput.connectors` into a flat
// `AgentTool[]` ready to push into pi-coding-agent's tool registry.
//
// As of the Composio cutover, BUILTIN is empty — every SaaS that used
// to ship as a per-connector createTools factory (Gmail / Slack / Google
// Docs / Google Calendar) now reaches the agent through Composio's
// hosted MCP server instead. See `apps/api/src/routes/composio.ts` and
// the `composio` entry that `enabled_mcp_specs` injects on every turn.
//
// The framework is intentionally left in place (the `BUILTIN` shape,
// `createCtx`, the snapshot-iteration loop) so a future non-Composio
// OAuth connector can be re-introduced by adding one workspace dep +
// one BUILTIN row, without touching the runner.

import type { AgentTool } from "@mariozechner/pi-agent-core";

import { log } from "../log.js";
import type { ConnectorsInput } from "../types.js";
import { createCtx } from "./ctx.js";
import type { ConnectorRefreshBridge, CreateToolsFn } from "./types.js";

// Built-in connector factory map. To add a future direct-OAuth
// connector: workspace dependency on `@corivo/connector-<id>` + one
// import + one row here. The id key MUST match the connector's
// `manifest.id` (and the Rust catalog's `manifest_for(id)`) — that's
// the wire between user-toggled-enabled and the JS factory.
const BUILTIN: Record<string, CreateToolsFn> = {};

/**
 * For each connector snapshot, instantiate a context and call its
 * factory; flatten the resulting `AgentTool[]`s. Unknown ids log +
 * skip. Errors during factory execution log + skip (we never let a
 * misbehaving connector kill the whole turn).
 */
export function registerConnectors(
  input: ConnectorsInput | undefined,
  bridge: ConnectorRefreshBridge,
): AgentTool<any>[] {
  if (!input || input.enabled.length === 0) {
    return [];
  }

  const tools: AgentTool<any>[] = [];
  for (const snapshot of input.enabled) {
    const factory = BUILTIN[snapshot.id];
    if (!factory) {
      log.warn("connector.loader.unknown_id", {
        connector_id: snapshot.id,
      });
      continue;
    }

    const ctx = createCtx({
      connectorId: snapshot.id,
      accountEmail: snapshot.account_email,
      grantedScopes: snapshot.granted_scopes,
      accessToken: snapshot.access_token,
      expiresAt: snapshot.expires_at,
      bridge,
    });

    try {
      const produced = factory(ctx);
      log.info("connector.loader.registered", {
        connector_id: snapshot.id,
        tools_count: produced.length,
        account_email: snapshot.account_email,
      });
      tools.push(...produced);
    } catch (err) {
      log.error("connector.loader.factory_threw", {
        connector_id: snapshot.id,
        error: (err as Error).message,
      });
      // continue — registering the rest is more useful than aborting
    }
  }

  return tools;
}
