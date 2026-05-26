// Capture upstream HTTP error response bodies so the agent can surface
// the provider's actual error text instead of openai-node's stripped
// "<status> status code (no body)" placeholder.
//
// Background: when a gateway (sub2api here) returns 4xx/5xx with a
// non-standard JSON body (e.g. `{"code":"API_KEY_QUOTA_EXHAUSTED",
// "message":"API key 额度已用完"}`), openai-node fails to find the
// expected `error.message` field and falls back to a generic string.
// pi-ai then forwards that generic string verbatim, so the user sees
// "429 status code (no body)" with no clue about what's actually wrong.
//
// Fix: install a `fetch` interceptor that, for chat/messages endpoints
// only, clones any 4xx/5xx response, reads its body as text, and stashes
// it in a module-scope holder. The agent-runner reads + clears that
// holder when it sees an `upstream_error` event and uses the captured
// body to compose a human-readable message.
//
// Scope: filtered to the LLM-call URLs (`/chat/completions` and
// `/messages`) so failures from tool HTTP calls don't poison the
// holder. Single-flight per turn is enough — the sidecar serves one
// chat turn at a time, and consume() clears the holder, so leaked
// state across turns can't happen.

type Capture = {
  status: number;
  body: string;
  url: string | undefined;
  requestBody: string | null;
};
let captured: Capture | null = null;
let installed = false;

const LLM_URL_PATTERNS = [
  "/chat/completions",
  "/messages",
  // OpenAI Responses API (`POST .../responses`) — used by openai-responses
  // adapter and the ChatGPT-subscription Codex endpoint
  // (`chatgpt.com/backend-api/codex/responses`). Without this entry a
  // 4xx from chatgpt.com surfaces as "<status> status code (no body)"
  // because the interceptor skips capturing the response.
  "/responses",
];

function looksLikeLlmCall(url: string | undefined): boolean {
  if (!url) return false;
  return LLM_URL_PATTERNS.some((pattern) => url.includes(pattern));
}

/**
 * Install the global fetch interceptor. Idempotent — calling more than
 * once is a no-op. Call once at sidecar boot (before pi-ai gets to
 * issue any requests).
 */
export function installUpstreamErrorCapture(): void {
  if (installed) return;
  installed = true;
  type FetchFn = typeof globalThis.fetch;
  type FetchInput = Parameters<FetchFn>[0];
  type FetchInit = Parameters<FetchFn>[1];

  function urlOf(input: FetchInput): string | undefined {
    if (typeof input === "string") return input;
    if (input instanceof URL) return input.toString();
    // Request-like with a `url` field. Avoid `instanceof Request`
    // because some runtimes (older Bun bundles) may not expose it.
    if (typeof input === "object" && input && "url" in input) {
      const u = (input as { url?: unknown }).url;
      return typeof u === "string" ? u : undefined;
    }
    return undefined;
  }

  const original = globalThis.fetch.bind(globalThis) as FetchFn;
  globalThis.fetch = (async (input: FetchInput, init?: FetchInit): Promise<Response> => {
    const url = urlOf(input);
    // Snapshot the request body *before* dispatch — we want it on hand
    // if the response turns out to be a 4xx/5xx. Best-effort: skipped
    // when the body is a stream we can't read without consuming, since
    // tee-ing back into the live request is finicky and openai-node
    // uses plain JSON strings in practice.
    const isLlmCall = looksLikeLlmCall(url);
    const requestBody = isLlmCall ? await snapshotRequestBody(input, init) : null;
    const response = await original(input, init);
    if (response.status >= 400 && isLlmCall) {
      // Clone so the downstream openai-node/pi-ai consumer still gets
      // a fresh body to parse. text() is unidirectional — without
      // clone(), the consumer would see an exhausted stream.
      try {
        const text = await response.clone().text();
        captured = { status: response.status, body: text, url, requestBody };
      } catch {
        // Body read failed (chunked stream issue, network reset, etc.).
        // Still record the status so callers can distinguish 401 from
        // other 4xx even when the body is unreadable.
        captured = { status: response.status, body: "", url, requestBody };
      }
    }
    return response;
  }) as typeof globalThis.fetch;
}

async function snapshotRequestBody(
  input: Parameters<typeof globalThis.fetch>[0],
  init: Parameters<typeof globalThis.fetch>[1],
): Promise<string | null> {
  try {
    // Path A: init.body present (most common — openai-node passes a
    // pre-stringified JSON here alongside url+method+headers).
    const initBody = init?.body;
    if (typeof initBody === "string") return initBody;
    if (initBody instanceof URLSearchParams) return initBody.toString();
    if (initBody instanceof ArrayBuffer) {
      return new TextDecoder("utf-8", { fatal: false }).decode(initBody);
    }
    if (ArrayBuffer.isView(initBody)) {
      const view = initBody as ArrayBufferView;
      const bytes = new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
      return new TextDecoder("utf-8", { fatal: false }).decode(bytes);
    }
    if (initBody) return `<unreadable body: ${Object.prototype.toString.call(initBody)}>`;

    // Path B: input is a Request — clone() so we don't drain the live
    // request stream that's about to be dispatched.
    if (
      typeof input === "object" &&
      input &&
      "clone" in input &&
      typeof (input as { clone?: unknown }).clone === "function"
    ) {
      const cloned = (input as Request).clone();
      return await cloned.text();
    }
  } catch {
    return null;
  }
  return null;
}

/**
 * Read + clear the last captured error response (status + body).
 * Returns `null` if no error response has been captured since the
 * last consume.
 *
 * Convention: agent-runner calls this exactly once per upstream_error
 * event, atomically draining the holder before composing the
 * user-facing message.
 */
export function consumeLastUpstreamError(): Capture | null {
  const value = captured;
  captured = null;
  return value;
}

/**
 * Best-effort extraction of a human-readable message from a captured
 * error body. Tries the common JSON shapes upstream gateways use:
 *
 *   - sub2api: `{"code":"...","message":"..."}`
 *   - OpenAI: `{"error":{"message":"...","type":"...","code":"..."}}`
 *   - Anthropic: `{"type":"error","error":{"type":"...","message":"..."}}`
 *   - Generic: `{"message":"..."}`, `{"detail":"..."}`, `{"error":"..."}`
 *
 * Falls back to the raw body when none match (truncated to keep the
 * UI bubble readable). Returns `null` if `body` is null / empty.
 */
export function extractUserFacingMessage(body: string | null): string | null {
  if (!body) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(body);
  } catch {
    // Non-JSON body — surface a truncated raw version so the user
    // still gets a hint of what came back.
    return body.length > 400 ? `${body.slice(0, 400)}…` : body;
  }
  if (!parsed || typeof parsed !== "object") {
    return body.length > 400 ? `${body.slice(0, 400)}…` : body;
  }
  const candidates: Array<unknown> = [
    (parsed as { message?: unknown }).message,
    (parsed as { error?: { message?: unknown } }).error?.message,
    (parsed as { error?: unknown }).error,
    (parsed as { detail?: unknown }).detail,
    (parsed as { error_description?: unknown }).error_description,
  ];
  for (const c of candidates) {
    if (typeof c === "string" && c.trim().length > 0) return c;
  }
  // None matched — fall back to JSON.stringify for visibility.
  const restringified = JSON.stringify(parsed);
  return restringified.length > 400
    ? `${restringified.slice(0, 400)}…`
    : restringified;
}
