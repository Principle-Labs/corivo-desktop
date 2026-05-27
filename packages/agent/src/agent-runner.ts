// Build and drive a pi `Agent` for a single turn, translating its events
// into corivo wire-schema NDJSON on stdout.
//
// Cross-turn history is persisted via pi-coding-agent's `SessionManager`
// jsonl store (spec §8.1). Each sidecar invocation:
//   1. Loads the thread's jsonl via `loadOrCreateSession(sessions_dir, thread_id)`.
//   2. Seeds `Agent.initialState.messages` from `sm.buildSessionContext()`
//      (which already applies any prior `compaction` entries).
//   3. Pre-appends the new user message to `sm`, then drives `agent.prompt`.
//   4. On every `turn_end` event, appends the assistant message + tool
//      results to `sm` so the jsonl mirrors `agent.state.messages`.
//   5. After `waitForIdle`, runs pi-coding-agent's `prepareCompaction` /
//      `compact` and writes a single `compaction` entry to `sm` so the
//      NEXT sidecar boot starts under the budget.
//
// The `transformContext` slot only carries `applyCompactionSafeguard` —
// in-memory hard truncation as a last-resort for single-turn explosions
// (a tool result that single-handedly blows past 0.95×window). Smart LLM
// summarization is now driven from agent_end here, NOT from per-LLM-round
// transformContext, because `appendCompaction` writes a one-shot persisted
// entry — every transformContext call would risk duplicate writes.

import { Agent, type AgentEvent } from "@mariozechner/pi-agent-core";
import {
  convertToLlm,
  type SessionManager,
} from "@mariozechner/pi-coding-agent";
import { streamSimple } from "@mariozechner/pi-ai";
import type {
  AssistantMessage,
  Message,
  Model,
  ToolResultMessage,
  UserMessage,
  Usage,
} from "@mariozechner/pi-ai";

/**
 * Custom User-Agent stamped on every upstream HTTP request.
 *
 * Why: the default Anthropic SDK / OpenAI SDK User-Agent
 * (`anthropic-ai-sdk-typescript/0.x` etc.) trips Cloudflare WAF on the
 * Corivo account-pool gateway (`https://llm.eiart.top`) — verified by
 * `curl` bisection, the request hits 403 "Your request was blocked."
 * solely because of the SDK UA. Overriding to a Corivo-branded UA lets
 * the request through. (The gateway can also whitelist the SDK UA on
 * its end; doing both is fine.)
 */
const CORIVO_AGENT_USER_AGENT = "corivo-agent/0.0.1 (+https://corivo.ai)";
const CORIVO_UA_HEADERS = { "user-agent": CORIVO_AGENT_USER_AGENT };

import type { ControlMessage, SidecarInput } from "./types.js";
import {
  consumeLastUpstreamError,
  extractUserFacingMessage,
} from "./upstream-error-capture.js";
import {
  buildCompactionModelAndConfig,
  buildModelAndStreamConfig,
} from "./auth.js";
import { buildSystemPrompt } from "./system-prompt.js";
import { resolveNativeTools } from "./native-tools/index.js";
import { wrapToolsWithTimeout } from "./tool-timeout.js";
import { buildMcpAgentTools } from "./mcp/runtime.js";
import { registerConnectors } from "./connector/loader.js";
import type { ConnectorRefreshBridge } from "./connector/types.js";
import { configureRpc } from "./rpc.js";
import { emit, type FinishReason } from "./events.js";
import { applyCompactionSafeguard } from "./extensions/compaction-safeguard.js";
import { runCrossTurnCompaction } from "./cross-turn-compaction.js";
import { inMemorySession, loadOrCreateSession } from "./session.js";
import { log } from "./log.js";

/**
 * Phase 1 mid-turn refresh stub. Per spec §4.2 the design supports
 * refreshing connector access_tokens during a long turn via stdio
 * NDJSON, but Phase 1 ships without that path:
 *   - typical turns are seconds to a few minutes;
 *   - access_token TTL is ~1 hour;
 *   - if a turn DOES outlast the TTL, the affected connector tool
 *     sees a 401 and the model gets a clean error to surface.
 * Phase 2 plumbs the actual stdio bridge — connector code is
 * unaffected because `ctx.fetch` already handles a rejecting bridge.
 */
const REJECTING_REFRESH_BRIDGE: ConnectorRefreshBridge = {
  async refresh(connectorId) {
    throw new Error(
      `connector ${connectorId} access_token expired mid-turn; mid-turn refresh is Phase 2 work`,
    );
  },
};

function nowMs(): number {
  return Date.now();
}

function elapsedMs(startedAt: number): number {
  return Math.max(0, Date.now() - startedAt);
}

function promptElapsedMs(promptStartedAt: number | null): number | null {
  return promptStartedAt === null ? null : elapsedMs(promptStartedAt);
}

function logPhase(
  phase: string,
  phaseStartedAt: number,
  totalStartedAt: number,
  fields: Record<string, unknown> = {},
): void {
  log.info("agent.phase", {
    phase,
    phase_ms: elapsedMs(phaseStartedAt),
    total_ms: elapsedMs(totalStartedAt),
    ...fields,
  });
}

function summarizePayload(payload: unknown): Record<string, unknown> {
  const obj = payload && typeof payload === "object"
    ? (payload as Record<string, unknown>)
    : {};
  const messages = obj.messages;
  const tools = obj.tools;
  return {
    model: obj.model,
    messages_count: Array.isArray(messages) ? messages.length : null,
    tools_count: Array.isArray(tools) ? tools.length : null,
    max_tokens: obj.max_tokens,
    stream: obj.stream,
    keys: Object.keys(obj).sort(),
  };
}

export async function runAgent(
  input: SidecarInput,
  controlStream: AsyncIterable<ControlMessage>,
  mockedModel?: Model<any>,
): Promise<void> {
  const totalStartedAt = nowMs();

  // Configure UDS RPC client (lazy-connect). Native tools may call this when
  // CORIVO_AGENT_PHASE_A_MOCK is unset.
  let phaseStartedAt = nowMs();
  configureRpc(input.rpc_socket);
  logPhase("rpc_configured", phaseStartedAt, totalStartedAt, {
    has_rpc_socket: input.rpc_socket.trim().length > 0,
  });

  phaseStartedAt = nowMs();
  const builtMain = buildModelAndStreamConfig(input);
  const builtCompaction = buildCompactionModelAndConfig(input);
  const { model: realModel, streamOptions } = builtMain;
  const model = mockedModel ?? realModel;
  logPhase("model_config_built", phaseStartedAt, totalStartedAt, {
    model_id: model.id,
    api: model.api,
    base_url: model.baseUrl,
    compaction_model_id: builtCompaction?.model.id ?? "(none)",
    using_mock: !!mockedModel,
  });

  phaseStartedAt = nowMs();
  const systemPrompt = buildSystemPrompt(input);
  logPhase("system_prompt_built", phaseStartedAt, totalStartedAt, {
    system_prompt_chars: systemPrompt.length,
    has_system_prompt_extra: !!input.system_prompt_extra?.trim(),
    has_focus_context: !!input.focus_context,
  });

  phaseStartedAt = nowMs();
  const nativeTools = resolveNativeTools(input.tools.native);
  logPhase("native_tools_resolved", phaseStartedAt, totalStartedAt, {
    native_tools: nativeTools.map((t) => t.name),
  });

  phaseStartedAt = nowMs();
  const { tools: mcpTools, shutdown: shutdownMcp } = await buildMcpAgentTools(
    input.tools.mcp_servers,
  );
  logPhase("mcp_tools_built", phaseStartedAt, totalStartedAt, {
    mcp_servers_count: input.tools.mcp_servers?.length ?? 0,
    mcp_tools_count: mcpTools.length,
  });

  phaseStartedAt = nowMs();
  const connectorTools = registerConnectors(
    input.connectors,
    REJECTING_REFRESH_BRIDGE,
  );
  logPhase("connector_tools_registered", phaseStartedAt, totalStartedAt, {
    connectors_count: input.connectors?.enabled.length ?? 0,
    connector_tools_count: connectorTools.length,
  });
  // Bound every tool call with a deadline (see tool-timeout.ts). A single
  // hung tool — observed in the wild as a `grep` over a pathological Windows
  // path that never returned — must not be able to freeze the turn.
  const tools = wrapToolsWithTimeout([
    ...nativeTools,
    ...mcpTools,
    ...connectorTools,
  ]);

  // Spec §8.1: open or create the jsonl session backed by Rust-supplied
  // sessions_dir (`$APPDATA/corivo-agent-sessions/`). Mock mode (Phase A
  // smoke test) and any input that omits sessions_dir falls back to an
  // in-memory session — same code path, no disk side effects.
  phaseStartedAt = nowMs();
  const sm: SessionManager =
    !mockedModel && input.sessions_dir
      ? loadOrCreateSession(input.sessions_dir, input.session_id)
      : inMemorySession();
  logPhase("session_opened", phaseStartedAt, totalStartedAt, {
    session_persisted: sm.isPersisted(),
    sessions_dir: input.sessions_dir ?? "(memory)",
  });

  // Resume from disk: pull the resolved (post-compaction) message list
  // and seed the Agent with it. `convertToLlm` from pi-coding-agent
  // expands `compactionSummary` / `branchSummary` AgentMessages into
  // plain user messages before they reach the LLM provider.
  phaseStartedAt = nowMs();
  const initialMessages = sm.buildSessionContext().messages;
  logPhase("session_context_built", phaseStartedAt, totalStartedAt, {
    initial_messages: initialMessages.length,
  });

  log.info("agent.configure", {
    model_id: model.id,
    api: model.api,
    base_url: model.baseUrl,
    using_mock: !!mockedModel,
    native_tools: nativeTools.map((t) => t.name),
    mcp_tools_count: mcpTools.length,
    connector_tools_count: connectorTools.length,
    total_tools: tools.length,
    compaction_enabled: !mockedModel,
    compaction_model_id: builtCompaction?.model.id ?? "(none)",
    thinking_level: input.model.thinking_level,
    session_persisted: sm.isPersisted(),
    initial_messages: initialMessages.length,
  });

  // Compaction hook policy (spec §8.3) split:
  //   - mid-turn safety net → `applyCompactionSafeguard` (in-memory truncation)
  //   - cross-turn smart summary → driven post-`waitForIdle` below, writes
  //     a one-shot `compaction` entry to sm.
  // Mocked-model runs (Phase A faux) skip the safeguard too — the faux
  // provider has a tiny synthetic transcript that never approaches the
  // 0.95 ceiling, and we don't want test runs touching the safeguard's
  // emit path either.
  const transformContext = mockedModel
    ? undefined
    : async (messages: any[]) =>
        applyCompactionSafeguard(messages, { main: builtMain });

  let promptStartedAt: number | null = null;
  let firstAgentEventLogged = false;
  let firstTextDeltaLogged = false;
  let firstThinkingDeltaLogged = false;
  let llmRequestIndex = 0;
  let lastLlmRequestStartedAt: number | null = null;
  const toolExecutionStartedAt = new Map<string, number>();

  // Agent state setup
  phaseStartedAt = nowMs();
  const agent = new Agent({
    initialState: {
      systemPrompt,
      model,
      thinkingLevel: input.model.thinking_level ?? "off",
      tools,
      messages: initialMessages,
    },
    // pi-coding-agent's convertToLlm turns its custom AgentMessage roles
    // (compactionSummary / branchSummary / bashExecution / custom) into
    // standard user/assistant/toolResult messages the LLM understands.
    // Plain Message[] passes through unchanged.
    convertToLlm,
    transformContext,
    getApiKey: () => streamOptions.apiKey,
    // Override the upstream HTTP User-Agent (see CORIVO_AGENT_USER_AGENT
    // doc-comment for why). pi-agent-core's streamFn slot receives the
    // model + context + SimpleStreamOptions; we forward verbatim plus
    // a headers override that gets merged into provider defaults.
    streamFn: mockedModel
      ? undefined
      : (m, ctx, opts) =>
          streamSimple(m, ctx, {
            ...(opts ?? {}),
            headers: { ...(opts?.headers ?? {}), ...CORIVO_UA_HEADERS },
          }),
    // pi-agent-core forwards these to pi-ai's stream() — they fire on the
    // upstream HTTP request body / response headers respectively. Mock
    // runs (faux provider) bypass HTTP, so these are no-ops there.
    onPayload: mockedModel
      ? undefined
      : (payload, m) => {
          llmRequestIndex += 1;
          lastLlmRequestStartedAt = nowMs();
          log.info("agent.http.request", {
            request_index: llmRequestIndex,
            model_id: m.id,
            base_url: m.baseUrl,
            prompt_ms: promptElapsedMs(promptStartedAt),
            total_ms: elapsedMs(totalStartedAt),
            ...summarizePayload(payload),
          });
          try {
            const sample = JSON.stringify(payload).slice(0, 200);
            log.trace("agent.http.payload", {
              model_id: m.id,
              base_url: m.baseUrl,
              payload_sample_200: sample,
            });
          } catch {
            // Non-serializable payload (e.g. embedded buffers) — skip.
          }
          return undefined;
        },
    onResponse: mockedModel
      ? undefined
      : (resp, m) => {
          log.info("agent.http.response", {
            request_index: llmRequestIndex,
            model_id: m.id,
            base_url: m.baseUrl,
            status: resp.status,
            request_ms: lastLlmRequestStartedAt === null
              ? null
              : elapsedMs(lastLlmRequestStartedAt),
            prompt_ms: promptElapsedMs(promptStartedAt),
            total_ms: elapsedMs(totalStartedAt),
          });
        },
  });
  logPhase("agent_constructed", phaseStartedAt, totalStartedAt, {
    total_tools: tools.length,
  });

  let turnIndex = 0;
  let usageTotal: Usage | undefined;
  let finishReason: FinishReason = "EndTurn";
  // Captured from assistant_message.errorMessage when pi-ai signals a
  // stopReason=error (e.g., upstream 4xx/5xx body). We forward this both
  // as an `error` wire event AND as a structured log line so "stream
  // finished with error" never bottoms out without a specific cause.
  let upstreamError:
    | { message: string; stopReason: string; code: string }
    | undefined;

  // Subscribe to translate pi-agent-core events → corivo wire envelope.
  const unsubscribe = agent.subscribe(async (event: AgentEvent) => {
    if (!firstAgentEventLogged) {
      firstAgentEventLogged = true;
      log.info("agent.event.first", {
        event_type: event.type,
        prompt_ms: promptElapsedMs(promptStartedAt),
        total_ms: elapsedMs(totalStartedAt),
      });
    }

    switch (event.type) {
      case "agent_start": {
        emit({
          type: "agent_start",
          data: { session_id: input.session_id, model: input.model.id },
        });
        return;
      }
      case "turn_start": {
        emit({ type: "turn_start", data: { turn_index: turnIndex } });
        turnIndex += 1;
        return;
      }
      case "message_update": {
        const ev = event.assistantMessageEvent;
        switch (ev.type) {
          case "text_delta":
            if (!firstTextDeltaLogged) {
              firstTextDeltaLogged = true;
              log.info("agent.first_text_delta", {
                prompt_ms: promptElapsedMs(promptStartedAt),
                total_ms: elapsedMs(totalStartedAt),
              });
            }
            emit({ type: "text_delta", data: { delta: ev.delta } });
            return;
          case "thinking_delta":
            if (!firstThinkingDeltaLogged) {
              firstThinkingDeltaLogged = true;
              log.info("agent.first_thinking_delta", {
                prompt_ms: promptElapsedMs(promptStartedAt),
                total_ms: elapsedMs(totalStartedAt),
              });
            }
            emit({ type: "thinking_delta", data: { delta: ev.delta } });
            return;
          case "toolcall_start": {
            // We only know the contentIndex at this point; partials of args
            // arrive in `toolcall_delta`. Phase A leaves tool_call_start
            // empty-shaped — Phase B will plumb id/name/partial.
            return;
          }
          case "toolcall_end": {
            const tc = ev.toolCall;
            log.debug("agent.tool_call_end", {
              tool_call_id: tc.id,
              name: tc.name,
              args_len: JSON.stringify(tc.arguments ?? {}).length,
            });
            emit({
              type: "tool_call_end",
              data: {
                tool_call_id: tc.id,
                name: tc.name,
                arguments: JSON.stringify(tc.arguments ?? {}),
              },
            });
            return;
          }
          default:
            return;
        }
      }
      case "tool_execution_start": {
        toolExecutionStartedAt.set(event.toolCallId, nowMs());
        log.info("agent.tool_execution.start", {
          tool_call_id: event.toolCallId,
          name: event.toolName,
          prompt_ms: promptElapsedMs(promptStartedAt),
          total_ms: elapsedMs(totalStartedAt),
        });
        emit({
          type: "tool_execution_start",
          data: {
            tool_call_id: event.toolCallId,
            name: event.toolName,
            args: event.args,
          },
        });
        return;
      }
      case "tool_execution_update": {
        emit({
          type: "tool_execution_update",
          data: {
            tool_call_id: event.toolCallId,
            partial_result: event.partialResult,
          },
        });
        return;
      }
      case "tool_execution_end": {
        const startedAt = toolExecutionStartedAt.get(event.toolCallId) ?? null;
        toolExecutionStartedAt.delete(event.toolCallId);
        log.info("agent.tool_execution.end", {
          tool_call_id: event.toolCallId,
          is_error: event.isError ?? false,
          duration_ms: startedAt === null ? null : elapsedMs(startedAt),
          prompt_ms: promptElapsedMs(promptStartedAt),
          total_ms: elapsedMs(totalStartedAt),
        });
        emit({
          type: "tool_execution_end",
          data: {
            tool_call_id: event.toolCallId,
            result: event.result,
            is_error: event.isError,
          },
        });
        return;
      }
      case "turn_end": {
        // Best-effort usage extraction from the assistant message
        const msg = event.message as Message | AssistantMessage;
        const usage =
          msg && "usage" in msg ? (msg.usage as Usage | undefined) : undefined;
        if (usage) {
          usageTotal = usage;
        }
        // Capture upstream error details when pi-ai surfaces them on
        // the assistant message. The errorMessage field carries the
        // provider's actual response body (e.g. "403 Your request was
        // blocked.") — or, when the gateway returns a non-OpenAI-shape
        // body, the unhelpful "<status> status code (no body)" fallback.
        // We consult the captured raw response body from the fetch
        // interceptor and, if it has a `message` field, replace the
        // generic fallback with it. Net effect: a 429 + sub2api body
        // `{"code":"API_KEY_QUOTA_EXHAUSTED","message":"API key 额度
        // 已用完"}` surfaces as the Chinese message instead of "429
        // status code (no body)".
        if (msg && "stopReason" in msg && msg.stopReason === "error") {
          const piMsg =
            (msg as AssistantMessage & { errorMessage?: string })
              .errorMessage ?? "(no errorMessage on assistant_message)";
          const capture = consumeLastUpstreamError();
          const extracted = extractUserFacingMessage(capture?.body ?? null);
          // Heuristic: when openai-node's stripped string is in play
          // (matches `XXX status code (no body)`), prefer the captured
          // body unconditionally — it carries strictly more signal.
          // Otherwise pi-ai already has useful text; only append the
          // body when it adds info.
          const looksStripped = /^\d{3} status code \(no body\)$/.test(
            piMsg.trim(),
          );
          let errMsg: string;
          if (extracted && looksStripped) {
            errMsg = extracted;
          } else if (extracted && !piMsg.includes(extracted)) {
            errMsg = `${piMsg} — ${extracted}`;
          } else {
            errMsg = piMsg;
          }
          // Tag 401s so the runner can react: gateway has rejected
          // our cached api_key (sub2api key rotation / corivo account
          // disable / session reaped). Rust catches `code: "auth_failed"`,
          // fires a `/auth/me` refresh, and tells the UI to invite a
          // resend with fresh creds.
          const upstreamStatus = capture?.status;
          const code = upstreamStatus === 401 ? "auth_failed" : "upstream_error";
          upstreamError = { message: errMsg, stopReason: "error", code };
          log.error("agent.upstream_error", {
            errorMessage: errMsg,
            piErrorMessage: piMsg,
            hasCapturedBody: capture !== null,
            upstreamStatus,
            upstreamUrl: capture?.url,
            requestBody: capture?.requestBody ?? null,
            responseBody: capture?.body ?? null,
            code,
            stopReason: msg.stopReason,
          });
        }
        // Mirror the assistant message + tool results into jsonl so the
        // next sidecar boot resumes mid-thread. We accept *all* assistant
        // messages, including stopReason=error/aborted — losing them on
        // resume would silently drop visible UI history that's already
        // been streamed to the user.
        try {
          sm.appendMessage(msg as Message);
          for (const tr of event.toolResults as ToolResultMessage[]) {
            sm.appendMessage(tr);
          }
        } catch (err) {
          // Persistence failure must NOT take down the turn — the user
          // already saw the response on the wire. Log loudly and continue.
          log.error("agent.session.append_failed", {
            message: (err as Error).message,
          });
        }
        emit({
          type: "turn_end",
          data: {
            assistant_message: event.message,
            tool_results: event.toolResults as ToolResultMessage[],
            usage,
          },
        });
        return;
      }
      case "agent_end": {
        // finish_reason determined separately from the final assistant
        // message stopReason (see below). Don't emit agent_end here — we
        // emit it after waitForIdle so finish_reason is set authoritatively.
        const last = lastAssistantMessage(event.messages);
        finishReason = mapStopReason(
          last?.stopReason,
          finishReason,
        );
        return;
      }
      default:
        return;
    }
  });

  // Wire control messages — currently only `cancel` is honored in Phase A.
  const controlPump = (async () => {
    for await (const msg of controlStream) {
      if (msg.type === "cancel") {
        finishReason = "Cancelled";
        agent.abort();
      } else if (msg.type === "steer") {
        // TODO(Phase B): forward to agent.steer({ role: "user", ... })
      } else if (msg.type === "compact") {
        // TODO(Phase C): trigger compaction extension manually
      }
    }
  })();
  // Don't await controlPump in foreground; it lives until stdin closes.

  // Build the user message ourselves so the SAME object lands in `sm`
  // (jsonl source-of-truth) and `agent.state.messages` (in-memory loop
  // input). Pre-appending to sm BEFORE prompt() means a crashed turn
  // still has the user message persisted — the next boot can re-attempt.
  const userMessage: UserMessage = {
    role: "user",
    content: input.user_message.content,
    timestamp: Date.now(),
  };
  try {
    sm.appendMessage(userMessage);
  } catch (err) {
    // Same persistence-failure-is-non-fatal stance as turn_end above.
    log.error("agent.session.user_append_failed", {
      message: (err as Error).message,
    });
  }

  try {
    log.info("agent.prompt.start", {
      session_id: input.session_id,
      user_message_chars: input.user_message.content.length,
      total_ms: elapsedMs(totalStartedAt),
    });
    promptStartedAt = nowMs();
    await agent.prompt(userMessage);
    log.info("agent.prompt.returned", {
      session_id: input.session_id,
      prompt_call_ms: elapsedMs(promptStartedAt),
      total_ms: elapsedMs(totalStartedAt),
    });
    const idleStartedAt = nowMs();
    await agent.waitForIdle();
    log.info("agent.wait_for_idle.done", {
      session_id: input.session_id,
      idle_ms: elapsedMs(idleStartedAt),
      total_ms: elapsedMs(totalStartedAt),
    });
    log.info("agent.prompt.done", {
      session_id: input.session_id,
      finish_reason: finishReason,
      turns: turnIndex,
      prompt_ms: promptElapsedMs(promptStartedAt),
      total_ms: elapsedMs(totalStartedAt),
      usage: usageTotal as Record<string, unknown> | undefined,
    });
  } finally {
    unsubscribe();
    const mcpShutdownStartedAt = nowMs();
    await shutdownMcp();
    log.info("agent.mcp.shutdown_done", {
      phase_ms: elapsedMs(mcpShutdownStartedAt),
      total_ms: elapsedMs(totalStartedAt),
    });
    // Best-effort drain of control stream — once stdin closes, the for-await
    // above exits naturally. We don't need to block on it.
    void controlPump;
  }

  // Cross-turn smart compaction (spec §8.3 primary path). Skip when:
  //   - mock mode (no real LLM available for summary)
  //   - run cancelled / errored (don't summarize a partial state)
  //   - compaction_model not configured (defensive)
  //   - persistence is in-memory (mock fallback) — nothing to compact across
  // TS sees `finishReason` as narrowed to its initializer "EndTurn" and
  // doesn't track that the `subscribe` callback mutates it; widen via
  // `string` so the comparison type-checks.
  const fr: string = finishReason;
  if (
    !mockedModel &&
    builtCompaction &&
    sm.isPersisted() &&
    fr !== "Cancelled" &&
    fr !== "Error"
  ) {
    const compactionStartedAt = nowMs();
    log.info("agent.compaction.start", {
      messages: agent.state.messages.length,
      main_model_id: builtMain.model.id,
      compaction_model_id: builtCompaction.model.id,
      total_ms: elapsedMs(totalStartedAt),
    });
    await runCrossTurnCompaction({
      sm,
      currentMessages: agent.state.messages,
      main: builtMain,
      compaction: builtCompaction,
    });
    log.info("agent.compaction.finished", {
      phase_ms: elapsedMs(compactionStartedAt),
      total_ms: elapsedMs(totalStartedAt),
    });
  } else {
    log.info("agent.compaction.skip_outer", {
      mocked_model: !!mockedModel,
      has_compaction_model: !!builtCompaction,
      session_persisted: sm.isPersisted(),
      finish_reason: fr,
      total_ms: elapsedMs(totalStartedAt),
    });
  }

  // Emit a specific `error` event so the FE renders the upstream
  // message instead of the generic "stream finished with error" fallback.
  // We do this before agent_end so use-chat.ts captures it into m.error
  // (its `case "finish"` only falls back to the generic string when
  // m.error is empty).
  // TS sees the initializer "EndTurn" and doesn't track that subscribe
  // mutates `finishReason`; cast through unknown to widen for this check.
  if ((finishReason as unknown as string) === "Error" && upstreamError) {
    emit({
      type: "error",
      data: {
        code: upstreamError.code,
        message: upstreamError.message,
        // 401 is recoverable: the Rust side will refresh /auth/me and
        // the user can re-send. Other upstream errors (quota out, 5xx,
        // model refusal) are not auto-recoverable without user action.
        recoverable: upstreamError.code === "auth_failed",
      },
    });
  }

  emit({
    type: "agent_end",
    data: {
      finish_reason: finishReason,
      usage_total: usageTotal,
    },
  });
  log.info("agent.run.done", {
    session_id: input.session_id,
    finish_reason: finishReason,
    turns: turnIndex,
    saw_first_text_delta: firstTextDeltaLogged,
    saw_first_thinking_delta: firstThinkingDeltaLogged,
    total_ms: elapsedMs(totalStartedAt),
  });
}

function lastAssistantMessage(
  messages: ReadonlyArray<unknown>,
): AssistantMessage | undefined {
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i] as { role?: string };
    if (m && m.role === "assistant") {
      return m as AssistantMessage;
    }
  }
  return undefined;
}

function mapStopReason(
  stopReason: AssistantMessage["stopReason"] | undefined,
  fallback: FinishReason,
): FinishReason {
  if (fallback === "Cancelled") return "Cancelled";
  switch (stopReason) {
    case "stop":
    case "length":
      return "EndTurn";
    case "toolUse":
      return "ToolUse";
    case "aborted":
      return "Cancelled";
    case "error":
      return "Error";
    default:
      return fallback;
  }
}
