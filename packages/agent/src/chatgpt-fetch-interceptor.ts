// Inject the special ChatGPT subscription headers + `store: false`
// body field on every request to `chatgpt.com/backend-api/codex/...`.
//
// Background: when `auth.mode === "chatgpt"` the sidecar builds an
// openai-responses Model pointed at `https://chatgpt.com/backend-api/codex`
// (see auth.ts). pi-ai / openai-node attaches `Authorization: Bearer
// <access_token>` automatically from `streamOptions.apiKey`, but the
// ChatGPT endpoint additionally requires three custom headers that
// neither openai-node nor pi-ai's streamSimple has any concept of:
//   * `ChatGPT-Account-Id`     — pulled from the OIDC id_token JWT
//                                 (`https://api.openai.com/auth.chatgpt_account_id`)
//   * `OpenAI-Beta`            — `responses=experimental`
//   * `originator`             — `corivo_desktop` (the Codex CLI uses
//                                 `codex_cli_rs`; Zed uses `zed`. We
//                                 declare our own so OpenAI can tell
//                                 third-party traffic apart from theirs.)
// The body also has to carry `store: false` — without it the chatgpt
// backend returns 400 demanding the client opt out of conversation
// storage. pi-ai never sets this field.
//
// Implementation shape: a global `fetch` interceptor installed once at
// sidecar boot. Same pattern as `upstream-error-capture.ts`. The
// account_id is configured per-turn via `setChatgptAccountId` (called
// from main.ts when it sees `input.auth.mode === "chatgpt"`). When the
// account_id is null the interceptor is a pass-through, so non-ChatGPT
// turns pay zero overhead beyond a URL prefix check.

import { log } from "./log.js";

const CODEX_URL_PREFIX = "https://chatgpt.com/backend-api/codex/";
const ORIGINATOR = "corivo_desktop";
const OPENAI_BETA = "responses=experimental";

let activeAccountId: string | null = null;
let installed = false;

/**
 * Set the ChatGPT account id the interceptor stamps on every Codex
 * request this turn. Pass `null` to disable (the interceptor then
 * becomes a no-op pass-through). Call once per turn from main.ts
 * after the SidecarInput JSON parse, when `input.auth.mode === "chatgpt"`.
 */
export function setChatgptAccountId(accountId: string | null): void {
  activeAccountId = accountId;
}

/**
 * Install the global fetch interceptor. Idempotent. Must run BEFORE
 * any LLM call goes out (pi-ai issues the first one as soon as the
 * Agent constructor returns), so the call site sits next to
 * `installUpstreamErrorCapture()` at the very top of main.ts.
 *
 * Ordering note: this wrapper layers on top of whatever fetch is
 * already in place. `installUpstreamErrorCapture` and this both bind
 * to the live `globalThis.fetch` at install time — the LAST one
 * installed wraps the others. Order in main.ts therefore matters:
 * upstream-error-capture first (so it sees the raw response), THIS
 * one second (so it gets to mutate headers + body before
 * upstream-error-capture sees them).
 */
export function installChatgptFetchInterceptor(): void {
  if (installed) return;
  installed = true;

  type FetchFn = typeof globalThis.fetch;
  type FetchInput = Parameters<FetchFn>[0];
  type FetchInit = Parameters<FetchFn>[1];

  function urlOf(input: FetchInput): string | undefined {
    if (typeof input === "string") return input;
    if (input instanceof URL) return input.toString();
    if (typeof input === "object" && input && "url" in input) {
      const u = (input as { url?: unknown }).url;
      return typeof u === "string" ? u : undefined;
    }
    return undefined;
  }

  const original = globalThis.fetch.bind(globalThis) as FetchFn;
  globalThis.fetch = (async (
    input: FetchInput,
    init?: FetchInit,
  ): Promise<Response> => {
    const url = urlOf(input);
    if (!activeAccountId || !url || !url.startsWith(CODEX_URL_PREFIX)) {
      return original(input, init);
    }

    const mutated = mutateForChatgpt(input, init, activeAccountId);
    return original(mutated.input, mutated.init);
  }) as typeof globalThis.fetch;
}

interface MutatedRequest {
  input: Parameters<typeof globalThis.fetch>[0];
  init: Parameters<typeof globalThis.fetch>[1];
}

function mutateForChatgpt(
  input: Parameters<typeof globalThis.fetch>[0],
  init: Parameters<typeof globalThis.fetch>[1],
  accountId: string,
): MutatedRequest {
  // openai-node always calls fetch with a string url + an init object.
  // The Request-object path is here only as a defensive fallback.
  const headers = new Headers(init?.headers ?? undefined);
  headers.set("ChatGPT-Account-Id", accountId);
  headers.set("OpenAI-Beta", OPENAI_BETA);
  headers.set("originator", ORIGINATOR);

  let nextBody: RequestInit["body"] = init?.body;
  let bodyMutated = false;
  if (typeof init?.body === "string") {
    const next = injectStoreFalse(init.body);
    if (next !== null) {
      nextBody = next;
      bodyMutated = true;
    }
  }

  const nextInit: RequestInit = {
    ...(init ?? {}),
    headers,
    body: nextBody,
  };

  log.debug("chatgpt.fetch.intercepted", {
    body_mutated: bodyMutated,
    has_string_body: typeof init?.body === "string",
  });

  return { input, init: nextInit };
}

/**
 * Body fields the chatgpt.com Codex endpoint doesn't accept (verified
 * empirically: sending any of these returns 400 with no body).
 *
 * `prompt_cache_key` + `prompt_cache_retention` are paid-tier Responses
 * API features that openai-node attaches by default — Codex CLI's own
 * request construction skips them. Strip before forwarding.
 */
const CHATGPT_UNSUPPORTED_FIELDS = [
  "prompt_cache_key",
  "prompt_cache_retention",
  // `max_output_tokens` is a paid Responses-API parameter — chatgpt.com
  // returns 400 `"Unsupported parameter: max_output_tokens"`. ChatGPT
  // subscriptions cap output via plan-level rate limits, not per-request.
  "max_output_tokens",
] as const;

/**
 * Parse `raw` as JSON, ensure `store: false`, hoist the system message
 * out of `input[]` into a top-level `instructions` field (the chatgpt.com
 * Codex endpoint rejects requests without it with `"Instructions are
 * required"`), and strip any fields the endpoint refuses. Returns the
 * re-stringified body, or `null` when the body isn't an object we can
 * sensibly mutate.
 *
 * pi-ai / openai-node send the system prompt as a normal `system` role
 * entry in `input[]` — that's how OpenAI's public Responses API takes
 * it. The ChatGPT-subscription variant requires it on `instructions`
 * instead, mirroring how Codex CLI builds its own request.
 */
function injectStoreFalse(raw: string): string | null {
  try {
    const parsed: unknown = JSON.parse(raw);
    if (
      parsed === null ||
      typeof parsed !== "object" ||
      Array.isArray(parsed)
    ) {
      return null;
    }
    const next: Record<string, unknown> = {
      ...(parsed as Record<string, unknown>),
      store: false,
    };
    for (const k of CHATGPT_UNSUPPORTED_FIELDS) {
      delete next[k];
    }
    hoistSystemToInstructions(next);
    return JSON.stringify(next);
  } catch {
    return null;
  }
}

/**
 * Mutate `body` so that any `role: "system"` entries in `input[]` are
 * concatenated into a single top-level `instructions` string and
 * removed from the input array. No-op when there's no system entry or
 * `instructions` is already populated (caller wins).
 *
 * Multiple system entries get joined with a blank line, mirroring
 * Codex CLI's behavior when a workflow stitches together base prompt +
 * persona + tools doc.
 */
function hoistSystemToInstructions(body: Record<string, unknown>): void {
  const input = body.input;
  if (!Array.isArray(input)) return;
  const systemTexts: string[] = [];
  const kept: unknown[] = [];
  for (const entry of input) {
    if (
      entry !== null &&
      typeof entry === "object" &&
      (entry as { role?: unknown }).role === "system"
    ) {
      const content = (entry as { content?: unknown }).content;
      const text = extractText(content);
      if (text) systemTexts.push(text);
      continue;
    }
    kept.push(entry);
  }
  if (systemTexts.length === 0) return;
  if (typeof body.instructions !== "string" || body.instructions.length === 0) {
    body.instructions = systemTexts.join("\n\n");
  }
  body.input = kept;
}

/**
 * Flatten an OpenAI Responses-API `content` field down to a single
 * string. Accepts the three shapes openai-node ever produces:
 *   * plain string
 *   * `[{ type: "input_text" | "output_text", text: "..." }, ...]`
 *   * a single `{ text: "..." }` object
 */
function extractText(content: unknown): string | null {
  if (typeof content === "string") return content;
  if (Array.isArray(content)) {
    const parts: string[] = [];
    for (const part of content) {
      if (
        part !== null &&
        typeof part === "object" &&
        typeof (part as { text?: unknown }).text === "string"
      ) {
        parts.push((part as { text: string }).text);
      }
    }
    return parts.length > 0 ? parts.join("") : null;
  }
  if (
    content !== null &&
    typeof content === "object" &&
    typeof (content as { text?: unknown }).text === "string"
  ) {
    return (content as { text: string }).text;
  }
  return null;
}
