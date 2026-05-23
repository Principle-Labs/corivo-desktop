// MCP runtime adapter — wires `mcporter` into the agent sidecar so each
// configured MCP server's tools become pi `AgentTool`s.
//
// Contract:
//   - Only servers passed in via `input.tools.mcp_servers` are connected.
//     mcporter's default host-config discovery (Cursor / Claude Desktop /
//     Codex / …) is bypassed by supplying `servers: [...]` to
//     `createRuntime` — when `servers` is provided mcporter skips the
//     `loadServerDefinitions` path entirely (see mcporter@0.10.1
//     `runtime.js::createRuntime`).
//   - Tools are exposed under `${serverName}__${toolName}` to avoid
//     collisions when two servers expose the same tool name.
//   - `allowed_tools` / `blocked_tools` on the spec are forwarded to
//     mcporter, which enforces them at the runtime layer. v0 default is
//     "all tools allowed" (omit `allowed_tools`).
//   - OAuth flows (for `transport: "http"` servers like Linear /
//     Notion) are handled by mcporter itself — the sidecar runs as a
//     child of Tauri, so any browser open + loopback callback happens
//     inside the sidecar process. The Rust host pre-warms this with a
//     one-shot `--bootstrap-mcp-oauth <id>` subcommand at connector
//     install time so the token is cached before normal agent boots.

import type { AgentTool, AgentToolResult } from "@mariozechner/pi-agent-core";
import type { Static, TSchema } from "typebox";
import { Type } from "typebox";
import {
  createRuntime,
  type Runtime,
  type ServerDefinition,
  type ServerToolInfo,
} from "mcporter";

import { log } from "../log.js";
import type { McpServerSpec } from "../types.js";

export interface BuiltMcpTools {
  tools: AgentTool<any>[];
  shutdown: () => Promise<void>;
}

function elapsedMs(startedAt: number): number {
  return Math.max(0, Date.now() - startedAt);
}

export async function buildMcpAgentTools(
  serverSpecs: McpServerSpec[] | undefined,
): Promise<BuiltMcpTools> {
  if (!serverSpecs || serverSpecs.length === 0) {
    return { tools: [], shutdown: async () => {} };
  }

  const totalStartedAt = Date.now();
  const definitions = serverSpecs.map(toServerDefinition);
  log.info("mcp.runtime.start", {
    servers_count: definitions.length,
    servers: definitions.map((def) => def.name),
  });

  const runtimeStartedAt = Date.now();
  const runtime = await createRuntime({ servers: definitions });
  log.info("mcp.runtime.created", {
    phase_ms: elapsedMs(runtimeStartedAt),
    total_ms: elapsedMs(totalStartedAt),
  });

  const tools: AgentTool<any>[] = [];
  for (const def of definitions) {
    let infos: ServerToolInfo[];
    const listStartedAt = Date.now();
    try {
      infos = await runtime.listTools(def.name, { includeSchema: true });
    } catch (err) {
      log.error("mcp.list_tools_failed", {
        server: def.name,
        error: (err as Error).message,
        phase_ms: elapsedMs(listStartedAt),
        total_ms: elapsedMs(totalStartedAt),
      });
      // Continue with other servers — one broken MCP must not abort
      // the whole sidecar boot.
      continue;
    }

    for (const info of infos) {
      tools.push(wrapMcpTool(runtime, def.name, info));
    }

    log.info("mcp.server_registered", {
      server: def.name,
      tools_count: infos.length,
      list_tools_ms: elapsedMs(listStartedAt),
      total_ms: elapsedMs(totalStartedAt),
    });
  }
  log.info("mcp.runtime.ready", {
    servers_count: definitions.length,
    tools_count: tools.length,
    total_ms: elapsedMs(totalStartedAt),
  });

  return {
    tools,
    shutdown: async () => {
      const shutdownStartedAt = Date.now();
      await runtime.close().catch((err) => {
        log.warn("mcp.shutdown_error", { error: (err as Error).message });
      });
      log.info("mcp.shutdown.done", {
        phase_ms: elapsedMs(shutdownStartedAt),
      });
    },
  };
}

// ─────────────────────────────────────────────── translator

function toServerDefinition(spec: McpServerSpec): ServerDefinition {
  const common = {
    name: spec.name,
    tokenCacheDir: spec.token_cache_dir,
    allowedTools: spec.allowed_tools,
    blockedTools: spec.blocked_tools,
  };

  if (spec.transport === "http") {
    return {
      ...common,
      command: {
        kind: "http",
        url: new URL(spec.url),
        // Forward static headers (e.g. `Authorization: Bearer <session>`)
        // when the host supplied them. mcporter's HttpCommand spec
        // accepts an optional `headers` record natively.
        ...(spec.headers && Object.keys(spec.headers).length > 0
          ? { headers: spec.headers }
          : {}),
      },
    };
  }

  // stdio
  return {
    ...common,
    command: {
      kind: "stdio",
      command: spec.command,
      args: spec.args ?? [],
      cwd: process.cwd(),
    },
    env: spec.env,
  };
}

// ─────────────────────────────────────────────── tool wrapper

function wrapMcpTool(
  runtime: Runtime,
  serverName: string,
  info: ServerToolInfo,
): AgentTool<TSchema> {
  // MCP `inputSchema` is JSON Schema; pi-agent-core's `AgentTool.parameters`
  // is typed as TSchema (TypeBox). The JSON Schema flows straight through
  // to the LLM provider — we just need to make TypeScript accept it.
  // `Type.Unsafe` is TypeBox's documented escape hatch for exactly this.
  const parameters = Type.Unsafe<unknown>(
    (info.inputSchema as object | undefined) ?? { type: "object" },
  );

  const qualifiedName = `${serverName}__${info.name}`;

  return {
    name: qualifiedName,
    label: info.name,
    description: info.description ?? `${info.name} (MCP tool from ${serverName})`,
    parameters,
    execute: async (
      _toolCallId: string,
      params: Static<typeof parameters>,
      signal?: AbortSignal,
    ): Promise<AgentToolResult<unknown>> => {
      if (signal?.aborted) {
        throw new Error(`${qualifiedName} aborted before dispatch`);
      }

      const raw = await runtime.callTool(serverName, info.name, {
        args: params as Record<string, unknown>,
      });
      const result = raw as {
        content?: McpContentBlock[];
        isError?: boolean;
        structuredContent?: unknown;
      };

      if (result.isError) {
        // Per AgentTool contract — throw on failure rather than encoding
        // the error inside `content`. The agent loop wraps thrown errors
        // into a tool-result message the model can read & self-correct.
        const text = extractText(result.content) ||
          `${qualifiedName} returned isError=true`;
        throw new Error(text);
      }

      return {
        content: convertContent(result.content ?? []),
        details: {
          mcp_server: serverName,
          mcp_tool: info.name,
          structured: result.structuredContent,
        },
      };
    },
  };
}

// ─────────────────────────────────────────────── content conversion

// Loose shape for MCP content blocks — we don't import @modelcontextprotocol/sdk
// types directly because mcporter's `callTool` returns them as `unknown`
// (intentionally — different MCP protocol versions emit slightly different
// shapes). We discriminate at runtime and stringify the long tail.
type McpContentBlock = Record<string, unknown> & { type?: unknown };

function convertContent(
  blocks: McpContentBlock[],
): AgentToolResult<unknown>["content"] {
  return blocks.map((b) => {
    if (b.type === "text" && typeof b.text === "string") {
      return { type: "text" as const, text: b.text };
    }
    if (
      b.type === "image" &&
      typeof b.data === "string" &&
      typeof b.mimeType === "string"
    ) {
      return { type: "image" as const, data: b.data, mimeType: b.mimeType };
    }
    // Audio / resource / resource_link / unknown → flatten to text so the
    // model still gets something. We can extend this later if a real
    // workload needs richer fidelity.
    return { type: "text" as const, text: JSON.stringify(b) };
  });
}

function extractText(blocks: McpContentBlock[] | undefined): string {
  if (!blocks) return "";
  return blocks
    .filter((b) => b.type === "text" && typeof b.text === "string")
    .map((b) => b.text as string)
    .join("\n");
}
