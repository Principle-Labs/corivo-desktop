// Map a SidecarInput to a pi-ai `Model<TApi>` plus stream options.
//
// Phase A note: pi-ai's `getModel(provider, modelId)` only knows about a
// hardcoded MODELS table — Corivo's "corivo:claude-*" ids aren't in there,
// nor are arbitrary BYOK ids. We instead construct `Model<TApi>` directly,
// because the Model interface is a plain TS interface (see
// `node_modules/@mariozechner/pi-ai/dist/types.d.ts`). The Provider field is
// constrained to "anthropic" | "openai" so pi-ai's built-in stream functions
// route correctly.
//
// Phase C TODO: when the backend `/v1/models` endpoint is live, plumb real
// `contextWindow` / `maxTokens` / `compaction_partner_id` through SidecarInput
// instead of hardcoded defaults. Phase B will also wire BYOK validation.

import type { Model, StreamOptions } from "@mariozechner/pi-ai";
import type { ApiShape, SidecarInput } from "./types.js";

const DEFAULT_ANTHROPIC_BASE_URL = "https://api.anthropic.com";
const DEFAULT_OPENAI_BASE_URL = "https://api.openai.com/v1";

// Phase A defaults — Phase C plumbs through real values from /v1/models.
const DEFAULT_ANTHROPIC_CONTEXT_WINDOW = 200_000;
const DEFAULT_ANTHROPIC_MAX_TOKENS = 8_192;
const DEFAULT_OPENAI_CONTEXT_WINDOW = 128_000;
const DEFAULT_OPENAI_MAX_TOKENS = 4_096;

export interface BuiltModel {
  model:
    | Model<"anthropic-messages">
    | Model<"openai-completions">
    | Model<"openai-responses">;
  streamOptions: StreamOptions;
}

export function buildModelAndStreamConfig(input: SidecarInput): BuiltModel {
  // ChatGPT subscription mode: hardcoded to openai-responses against
  // `chatgpt.com/backend-api/codex` (the only endpoint the ChatGPT
  // OAuth token can actually call). The api_shape on input is
  // already "openai_responses" — Rust runtime resolution forces it —
  // but we override base_url defensively so a misconfigured stale
  // input can't accidentally hit api.openai.com with a ChatGPT
  // token (which would 401).
  if (input.auth.mode === "chatgpt") {
    return buildModel(
      input.model.id,
      "openai_responses",
      "https://chatgpt.com/backend-api/codex",
      input.auth.token,
      input.model.thinking_level !== "off" && input.model.thinking_level !== undefined,
    );
  }
  return buildModel(
    input.model.id,
    input.model.api_shape,
    input.auth.base_url,
    input.auth.token,
    input.model.thinking_level !== "off" && input.model.thinking_level !== undefined,
  );
}

/**
 * Build a BuiltModel for the cheap compaction LLM (spec §7.6 / §8.3).
 *
 * - CorivoProxy: backend gateway picks the cheap model via
 *   `compaction_partner_id`; sidecar just plumbs id + api_shape and
 *   reuses the same auth (`base_url` + `token`) the main model uses.
 * - BYOK: spec §7.6 mandates a hardcoded default per shape — Anthropic
 *   → claude-haiku-4-5, OpenAI → gpt-4o-mini. The Rust runner already
 *   sends `compaction_model` matching this contract; if it's missing
 *   we fall back to the same defaults defensively.
 *
 * Returns `null` when the input intentionally omits `compaction_model`
 * (only happens in tests) — callers are expected to short-circuit
 * compaction in that case.
 */
export function buildCompactionModelAndConfig(
  input: SidecarInput,
): BuiltModel | null {
  const spec = input.compaction_model;
  if (!spec) return null;
  return buildModel(
    spec.id,
    spec.api_shape,
    input.auth.base_url,
    input.auth.token,
    // Compaction never benefits from extended thinking — keep it cheap.
    false,
  );
}

function buildModel(
  id: string,
  api_shape: ApiShape,
  base_url: string | undefined,
  token: string,
  reasoning: boolean,
): BuiltModel {
  if (api_shape === "anthropic") {
    const baseUrl = base_url ?? DEFAULT_ANTHROPIC_BASE_URL;
    const model: Model<"anthropic-messages"> = {
      id,
      name: id,
      api: "anthropic-messages",
      provider: "anthropic",
      baseUrl,
      reasoning,
      input: ["text", "image"],
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
      contextWindow: DEFAULT_ANTHROPIC_CONTEXT_WINDOW,
      maxTokens: DEFAULT_ANTHROPIC_MAX_TOKENS,
    };
    return { model, streamOptions: { apiKey: token } };
  }

  if (api_shape === "openai") {
    // pi-ai feeds this string verbatim into the openai-node client's
    // `baseURL` option. That client only appends `/chat/completions` —
    // it does NOT prepend `/v1`. So a host without `/v1` (e.g.
    // `https://llm.eiart.top`) ends up hitting `${host}/chat/completions`,
    // which many gateways route to their SPA index.html and return
    // 200 + HTML. pi-ai then sees no parseable chunks → empty reply
    // in the bubble. Normalize once here so admins can store the
    // human-friendly host in the catalog and not worry about the
    // trailing-/v1 convention. (Anthropic adapter is unaffected — it
    // builds its own `/v1/messages` path.)
    const baseUrl = ensureOpenAIV1(base_url ?? DEFAULT_OPENAI_BASE_URL);
    const model: Model<"openai-completions"> = {
      id,
      name: id,
      api: "openai-completions",
      // sub2api normalizes Azure / Gemini into the openai shape, so the
      // literal `"openai"` provider here covers every upstream we surface
      // through this branch today. The DeepSeek reasoning_content
      // contract is keyed off `baseUrl.includes("deepseek.com")` inside
      // pi-ai's adapter — it activates automatically when sub2api points
      // at a DeepSeek-backed alias, no extra wiring needed here.
      provider: "openai",
      baseUrl,
      // Forward the resolved `reasoning` flag from the call site rather
      // than hardcoding `false`. With it pinned off we would skip pi-ai's
      // DeepSeek reasoning_content round-trip, which is exactly the
      // failure mode diagnosed earlier: tool-use turn 1 succeeds, turn 2
      // 400s because the assistant history is missing reasoning_content.
      // The Rust runner derives this flag from `ModelCapabilities.reasoning`
      // for managed models and from `ThinkingLevel != Off` for BYOK.
      reasoning,
      input: ["text", "image"],
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
      contextWindow: DEFAULT_OPENAI_CONTEXT_WINDOW,
      maxTokens: DEFAULT_OPENAI_MAX_TOKENS,
    };
    return { model, streamOptions: { apiKey: token } };
  }

  if (api_shape === "openai_responses") {
    // OpenAI Responses API (`POST {baseUrl}/responses`). 用于 GPT-5 系列
    // 等只在 Responses 形态稳定支持工具调用的 model —— 这些 model 走
    // openai-completions 时,sub2api 在 chat→responses 翻译里会把
    // `tool_calls[].id="call_xxx"` 改写成 `function_call.call_id="fcxxx"`
    // 且与 function_call_output 配对断裂,产生
    // `400 No tool call found for function call output with call_id ...`。
    // 走 Responses API 让客户端绕过那一层翻译,call_id 端到端一致。
    //
    // baseUrl 归一化与 openai-completions 完全一致 ——`ensureOpenAIV1`
    // 保证落到 `${host}/v1`,openai-node 客户端把 `/responses` 接在后面。
    const baseUrl = ensureOpenAIV1(base_url ?? DEFAULT_OPENAI_BASE_URL);
    const model: Model<"openai-responses"> = {
      id,
      name: id,
      api: "openai-responses",
      provider: "openai",
      baseUrl,
      reasoning,
      input: ["text", "image"],
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
      contextWindow: DEFAULT_OPENAI_CONTEXT_WINDOW,
      maxTokens: DEFAULT_OPENAI_MAX_TOKENS,
    };
    return { model, streamOptions: { apiKey: token } };
  }

  // exhaustive — keeps TS honest if we add a new api_shape
  const _exhaustive: never = api_shape satisfies never as never;
  throw new Error(`unsupported api_shape: ${String(_exhaustive)}`);
}

/**
 * Ensure an openai-shape base URL ends with `/v1`. Handles trailing
 * slashes and pre-normalized inputs idempotently. Does NOT add it when
 * any path segment is already present (e.g. `/openai/v1`,
 * `/api/v3/openai/v1/`) — that's the admin saying "I know what I'm
 * doing, route here".
 */
function ensureOpenAIV1(rawBaseUrl: string): string {
  try {
    const url = new URL(rawBaseUrl);
    // Strip trailing slash for stable comparison.
    const pathname = url.pathname.replace(/\/+$/, "");
    if (pathname === "" || pathname === "/") {
      url.pathname = "/v1";
      return url.toString().replace(/\/+$/, "");
    }
    // Some path present — trust the admin's intent. Strip trailing
    // slash for consistency but otherwise leave alone.
    url.pathname = pathname;
    return url.toString().replace(/\/+$/, "");
  } catch {
    // Malformed — pi-ai will reject it loudly later. Surface as-is.
    return rawBaseUrl;
  }
}

export function defaultBaseUrlFor(api_shape: ApiShape): string {
  return api_shape === "anthropic"
    ? DEFAULT_ANTHROPIC_BASE_URL
    : DEFAULT_OPENAI_BASE_URL;
}

/**
 * Validate that the BYOK model id is real by hitting the provider's models
 * list endpoint. Phase B wires this in `main.ts`; Phase A keeps it as a
 * standalone helper so smoke tests don't need network.
 *
 * TODO(Phase B): call from `main.ts` on startup when `auth.mode === "byok"`.
 */
export async function validateByokModel(input: SidecarInput): Promise<void> {
  if (input.auth.mode !== "byok") return;
  const { api_shape } = input.model;
  const base = input.auth.base_url ?? defaultBaseUrlFor(api_shape);
  const url =
    api_shape === "anthropic" ? `${base}/v1/models` : `${base}/models`;
  const headers: Record<string, string> =
    api_shape === "anthropic"
      ? {
          "x-api-key": input.auth.token,
          "anthropic-version": "2023-06-01",
        }
      : { authorization: `Bearer ${input.auth.token}` };

  const resp = await fetch(url, { headers });
  if (!resp.ok) {
    throw new Error(
      `byok_validation_failed: provider models endpoint returned ${resp.status}`,
    );
  }
  const json = (await resp.json()) as { data?: Array<{ id: string }> };
  const ids = json.data?.map((m) => m.id) ?? [];
  if (!ids.includes(input.model.id)) {
    throw new Error(
      `byok_model_not_found: model "${input.model.id}" not in provider's available list`,
    );
  }
}
