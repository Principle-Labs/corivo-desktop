// Sidecar input schema — matches agent-sidecar-spec §5.1.
//
// Produced by Rust `services/exec_agent/runner.rs` (Phase B); consumed by
// `main.ts` over stdin (one JSON object terminated by newline, then stdin
// stays open for control messages — see §5.3).

export type ApiShape = "anthropic" | "openai" | "openai_responses";

export type ThinkingLevel =
  | "off"
  | "minimal"
  | "low"
  | "medium"
  | "high"
  | "xhigh";

export type AuthMode = "corivo_proxy" | "byok";

export interface InputImage {
  /** Either base64-encoded data or a file path. Phase A only carries it through. */
  data: string;
  mime_type: string;
}

export interface UserMessageInput {
  role: "user";
  content: string;
  images?: InputImage[];
}

export interface FocusContext {
  frame_id?: string;
  summary?: string;
  primary_text?: string;
  selection?: string;
}

export interface ModelSpec {
  /** Either Corivo internal id (e.g. "corivo:claude-sonnet-4-6") or BYOK provider id. */
  id: string;
  api_shape: ApiShape;
  thinking_level?: ThinkingLevel;
}

export interface AuthSpec {
  mode: AuthMode;
  base_url?: string;
  token: string;
}

/**
 * One MCP server the sidecar should connect to.
 *
 * v0 supports two transports:
 *  - `http`: remote SSE endpoint (vendor-hosted MCP, e.g. Linear's
 *    `https://mcp.linear.app/sse`). OAuth is handled by mcporter
 *    against the vendor's auth server.
 *  - `stdio`: locally-spawned MCP server (community / self-hosted).
 *
 * `token_cache_dir` is Corivo-controlled (per-spec, usually
 * `$APPDATA/corivo-mcp-tokens/<name>/`) so token state is isolated
 * from anything mcporter would discover from Cursor / Claude Desktop
 * configs and so `disconnect` can wipe it cleanly.
 *
 * `allowed_tools` defaults to "all" when omitted — by product
 * decision, vendor MCPs install fully-open and users can prune later.
 *
 * Wire-level naming is snake_case to match every other Rust→sidecar
 * input field on `SidecarInput`.
 */
export type McpServerSpec =
  | {
      name: string;
      transport: "http";
      url: string;
      /**
       * Static HTTP headers to attach to every JSON-RPC request to this
       * server. Use `Authorization: Bearer <token>` for backends that
       * don't carry credentials in the URL — notably Corivo's own
       * `/v1/composio/mcp` proxy, which authenticates via the user's
       * session bearer.
       */
      headers?: Record<string, string>;
      token_cache_dir?: string;
      allowed_tools?: string[];
      blocked_tools?: string[];
    }
  | {
      name: string;
      transport: "stdio";
      command: string;
      args?: string[];
      env?: Record<string, string>;
      token_cache_dir?: string;
      allowed_tools?: string[];
      blocked_tools?: string[];
    };

export interface ToolsSpec {
  native: string[];
  mcp_servers?: McpServerSpec[];
}

/**
 * Per-connector token + scope snapshot. Rust writes one entry per
 * enabled + connected connector at agent spawn time. The agent's
 * connector loader (see `connector/loader.ts`) consumes these to
 * register each connector's `createTools(ctx)` factory.
 *
 * Mirrors the Rust `ConnectorTokenSnapshot` ts-rs export.
 */
export interface ConnectorSnapshotInput {
  id: string;
  account_email: string | null;
  granted_scopes: string[];
  access_token: string;
  /** RFC 3339, or null if the IdP didn't tell us. */
  expires_at: string | null;
}

export interface ConnectorsInput {
  enabled: ConnectorSnapshotInput[];
}

export interface SidecarInput {
  session_id: string;
  user_message: UserMessageInput;
  focus_context?: FocusContext;
  system_prompt_extra?: string;
  model: ModelSpec;
  compaction_model?: ModelSpec;
  auth: AuthSpec;
  tools: ToolsSpec;
  /** UDS path Rust listens on for native-tool callbacks (§5.4). */
  rpc_socket: string;
  /**
   * Spec §8.1: directory where pi-coding-agent's `SessionManager`
   * persists conversation jsonl files. Rust resolves this from
   * `$APPDATA/corivo-agent-sessions/` and ensures it exists before
   * spawn. Optional only because mock-mode / unit-test fixtures may
   * omit it — the sidecar falls back to an in-memory session.
   */
  sessions_dir?: string;
  /**
   * App-private bundled skills root. In packaged builds Rust resolves
   * this from `<resource_dir>/bundled-skills/`; in `app:dev` it
   * falls back to the repo source `apps/desktop/bundled-skills/`.
   * Optional — when absent (mock-mode, missing resource), the sidecar
   * just doesn't advertise any bundled skills; user-only skill
   * sources still load.
   */
  bundled_skills_dir?: string;
  /**
   * External-service connectors (Gmail / Notion / Slack / …) enabled
   * for this turn. Optional — when omitted, the agent simply registers
   * no connector tools. Tokens here are spawn-time snapshots; if a
   * turn outlasts an `access_token` TTL, the agent emits a
   * `host_request:refresh_connector_token` over stdout NDJSON and
   * Rust replies on stdin (see `services::connector::runtime`).
   */
  connectors?: ConnectorsInput;
}

// Control messages on stdin after the first JSON (§5.3).
export type ControlMessage =
  | { type: "cancel" }
  | { type: "steer"; content: string }
  | { type: "compact" };

export class SidecarError extends Error {
  readonly code: string;
  readonly recoverable: boolean;
  constructor(code: string, message: string, recoverable = false) {
    super(message);
    this.code = code;
    this.recoverable = recoverable;
    this.name = "SidecarError";
  }
}
