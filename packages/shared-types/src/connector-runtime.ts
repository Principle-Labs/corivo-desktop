// Hand-written types for the connector runtime contract — the surface
// connector packages (`packages/connector-<id>/`) consume, and the agent
// loader provides.
//
// Lives here (rather than in `@corivo/agent`) because both sides of the
// contract need it: `@corivo/agent` _provides_ the ctx; each connector
// package _consumes_ it. Hosting the type in the agent package would
// require each connector to depend on `@corivo/agent`, which depends
// on the connectors — a cycle pnpm correctly refuses to resolve.
//
// This file is NOT ts-rs-generated; the surrounding `generated/`
// directory is. Keep this distinction so future regenerations of
// ts-rs bindings don't accidentally clobber hand-written types.

/**
 * What a connector's tool implementation receives. `ctx.fetch` is the
 * only call surface for HTTP — it automatically injects the OAuth
 * `Authorization: Bearer ...` header for the connector's connected
 * account, and on 401 transparently refreshes the access_token and
 * retries the request once. Connectors must NEVER override the
 * Authorization header (the host strips/replaces it).
 *
 * The host never hands out the raw access_token string to connector
 * code — it's an implementation detail of `fetch`. That decoupling is
 * what lets a future Phase 2 swap the in-process implementation for a
 * Worker-Thread message-passing one without touching connector code.
 */
export interface ConnectorCtx {
  /** Stable connector id, matches manifest.id. */
  readonly connectorId: string;
  /** Display email for logs / error messages. May be null on services
   *  that don't expose an email (Slack team, Notion workspace, …). */
  readonly accountEmail: string | null;
  /** Scopes the IdP actually granted. Connectors should consult this
   *  before calling endpoints that require scopes the user didn't
   *  grant, and surface a friendly error rather than hit 403. */
  readonly grantedScopes: string[];

  /** Authorized HTTP fetch. Same signature as global `fetch`. */
  fetch(url: string, init?: RequestInit): Promise<Response>;

  /** Structured log. Routed to agent stdout (NDJSON) by the host. */
  log(
    level: "debug" | "info" | "warn" | "error",
    msg: string,
    meta?: Record<string, unknown>,
  ): void;
}
