// Sidecar entry point.
//
// 1. Read one line of JSON from stdin (matches §5.1 SidecarInput).
// 2. Build pi `Agent` with native + mcp tools, run a turn.
// 3. Subscribe to agent events and translate to corivo NDJSON wire schema
//    on stdout (§5.2).
// 4. Keep stdin open for control messages (cancel/steer/compact, §5.3).
// 5. On agent_end emit envelope and exit.

// MUST be the FIRST import. Sets `PI_PACKAGE_DIR` so pi-coding-agent's
// top-level `JSON.parse(readFileSync(packageJson))` resolves a real
// file when running as a `bun --compile` binary. See the file's own
// header for the full rationale.
import "./bootstrap-pi-env.js";

import "@mariozechner/pi-ai"; // side-effect: register built-in providers

// Install a global `fetch` wrapper that records 4xx/5xx response
// bodies for chat/messages endpoints. agent-runner consumes the
// captured body when pi-ai surfaces a stopReason=error so the user
// sees the gateway's actual message (e.g. "API key 额度已用完")
// instead of openai-node's "<status> status code (no body)" fallback.
// Must run BEFORE any LLM call goes out.
import { installUpstreamErrorCapture } from "./upstream-error-capture.js";
installUpstreamErrorCapture();

// "Sign in with ChatGPT" header + body injector (chatgpt-fetch-
// interceptor.ts module-doc has the full why). Installed second so
// it gets to mutate request headers + body BEFORE
// upstream-error-capture observes the eventual response. Activated
// per-turn via setChatgptAccountId once we know we're in chatgpt mode.
import {
  installChatgptFetchInterceptor,
  setChatgptAccountId,
} from "./chatgpt-fetch-interceptor.js";
installChatgptFetchInterceptor();

import {
  registerFauxProvider,
  fauxText,
  fauxAssistantMessage,
  type Model,
} from "@mariozechner/pi-ai";

import { runAgent } from "./agent-runner.js";
import { runBootstrapMcpOauth, runClearMcpOauth } from "./mcp/bootstrap.js";
import type { ControlMessage, SidecarInput } from "./types.js";
import { emit } from "./events.js";
import { isMockMode } from "./mock.js";
import { validateByokModel } from "./auth.js";
import { log, redact } from "./log.js";

/** Subcommand argv flag for the one-shot MCP OAuth bootstrap. */
const BOOTSTRAP_MCP_OAUTH_FLAG = "--bootstrap-mcp-oauth";
/** Subcommand argv flag for the one-shot MCP OAuth cache wipe
 *  (counterpart to bootstrap; runs at connector 断开 time). */
const CLEAR_MCP_OAUTH_FLAG = "--clear-mcp-oauth";

function elapsedMs(startedAt: number): number {
  return Math.max(0, Date.now() - startedAt);
}

interface ParsedInput {
  input: SidecarInput;
  controlStream: AsyncIterable<ControlMessage>;
}

async function readSidecarInput(): Promise<ParsedInput> {
  const startedAt = Date.now();
  // Bun --compile 1.3.x has a hostile stdin bug: every API we tried
  // (`for await of process.stdin`, `.on("data")`, `Bun.stdin.stream()`,
  // even `Bun.file(0).text()`) hangs forever inside the compiled binary
  // even after the parent closes the write end. Same source runs fine
  // under `bun run`, breaks under `bun build --compile`. Reproduced on
  // bun 1.3.11 darwin-arm64 / darwin-x64.
  //
  // Workaround: don't use stdin. Rust writes the §5.1 SidecarInput
  // payload to a temp file path and passes the path as `argv[2]`.
  // Reading a regular file works correctly in compile mode.
  //
  // Tradeoff: lose the §5.3 control channel (cancel/steer/compact)
  // multiplexed on stdin. None of those are wired to the UI yet, so
  // the loss is theoretical for now. When we need them, they go over
  // the same UDS socket the sidecar already uses for native-tool
  // callbacks (it's bidirectional).
  // Bun's compiled binary layout: argv[0] = binary, argv[1+] = user args.
  // In `bun run script.ts` mode, argv[0]=bun, argv[1]=script.ts, argv[2+]=user args.
  // Take the LAST arg so we work in both modes — the Rust runner only
  // ever passes one user arg (the input file path).
  const inputPath = process.argv[process.argv.length - 1];
  if (
    !inputPath ||
    inputPath === process.argv[0] ||
    inputPath.endsWith("main.ts")
  ) {
    throw new Error(
      "sidecar: missing input file path arg — Rust runner must pass the temp file path",
    );
  }
  const raw = await Bun.file(inputPath).text();
  const trimmed = raw.trim();
  if (!trimmed) {
    throw new Error(`sidecar: input file at ${inputPath} is empty`);
  }
  const input = JSON.parse(trimmed) as SidecarInput;
  log.info("sidecar.input_loaded", {
    input_path: inputPath,
    input_bytes: raw.length,
    read_parse_ms: elapsedMs(startedAt),
    session_id: input.session_id,
    model_id: input.model.id,
    native_tools_count: input.tools.native.length,
    mcp_servers_count: input.tools.mcp_servers?.length ?? 0,
    connectors_count: input.connectors?.enabled.length ?? 0,
  });

  // Empty control stream — see comment above for why.
  const controlStream: AsyncIterable<ControlMessage> = {
    [Symbol.asyncIterator]() {
      return {
        async next(): Promise<IteratorResult<ControlMessage>> {
          return { value: undefined, done: true };
        },
      };
    },
  };

  return { input, controlStream };
}

async function main(): Promise<number> {
  const mainStartedAt = Date.now();
  let input: SidecarInput;
  let controlStream: AsyncIterable<ControlMessage>;
  try {
    const inputStartedAt = Date.now();
    const parsed = await readSidecarInput();
    input = parsed.input;
    controlStream = parsed.controlStream;
    log.info("sidecar.input_ready", {
      session_id: input.session_id,
      phase_ms: elapsedMs(inputStartedAt),
      total_ms: elapsedMs(mainStartedAt),
    });
  } catch (err) {
    log.error("sidecar.input_parse_failed", {
      message: String((err as Error).message),
      total_ms: elapsedMs(mainStartedAt),
    });
    emit({
      type: "error",
      data: {
        code: "input_parse_failed",
        message: String((err as Error).message),
        recoverable: false,
      },
    });
    return 1;
  }

  // Sidecar boot trace — the Rust drain_stderr forwarder dispatches this
  // into the daily log file as `target=corivo_agent level=info`.
  log.info("sidecar.start", {
    session_id: input.session_id,
    model_id: input.model.id,
    api_shape: input.model.api_shape,
    thinking_level: input.model.thinking_level,
    auth_mode: input.auth.mode,
    auth_base_url: input.auth.base_url ?? "(default)",
    auth_token: redact(input.auth.token),
    has_focus_context: !!input.focus_context,
    user_message_chars: input.user_message.content.length,
    native_tools: input.tools.native,
    mcp_servers_count: input.tools.mcp_servers?.length ?? 0,
    connectors_count: input.connectors?.enabled.length ?? 0,
    rpc_socket: input.rpc_socket,
    mock_mode: isMockMode(),
    total_ms: elapsedMs(mainStartedAt),
  });

  // Phase A faux mode: register a faux pi-ai provider so the Agent can
  // produce realistic streaming events without real API credentials.
  let mockedModel: Model<string> | undefined;
  if (isMockMode()) {
    const reg = registerFauxProvider({
      api: "faux",
      provider: "faux",
      models: [
        {
          id: input.model.id,
          name: input.model.id,
          contextWindow: 128000,
          maxTokens: 16384,
        },
      ],
    });
    reg.setResponses([
      fauxAssistantMessage(
        [
          fauxText(
            "Hello from the Corivo Agent sidecar Phase A demo. Streaming a few sentences via the faux pi-ai provider so you can see text_delta events on the wire.",
          ),
        ],
        { stopReason: "stop" },
      ),
    ]);
    mockedModel = reg.getModel(input.model.id);
  }

  // Activate the ChatGPT subscription fetch interceptor for this turn.
  // The interceptor stays installed across turns but is a pass-through
  // until we hand it an account_id. Refuse to start the turn if the
  // account_id field is missing — without it chatgpt.com/backend-api/codex
  // would just 401 every request.
  if (input.auth.mode === "chatgpt") {
    if (!input.auth.account_id || input.auth.account_id.trim().length === 0) {
      log.error("sidecar.chatgpt.missing_account_id", {});
      emit({
        type: "error",
        data: {
          code: "chatgpt_missing_account_id",
          message:
            "ChatGPT mode is selected but `auth.account_id` is empty — re-run sign-in to fix the cached credentials.",
          recoverable: false,
        },
      });
      emit({ type: "agent_end", data: { finish_reason: "Error" } });
      return 1;
    }
    setChatgptAccountId(input.auth.account_id);
    log.info("sidecar.chatgpt.activated", {
      account_id_len: input.auth.account_id.length,
    });
  } else {
    setChatgptAccountId(null);
  }

  // Phase C §7.7: BYOK model id validation. Done up front so a typo
  // surfaces as a single clean error event rather than a confusing 4xx
  // mid-stream. Mock mode (Phase A smoke test) skips this — the faux
  // provider has no `/models` endpoint to query.
  if (!isMockMode() && input.auth.mode === "byok") {
    const validationStartedAt = Date.now();
    log.debug("sidecar.byok_validation.start", {
      model_id: input.model.id,
      total_ms: elapsedMs(mainStartedAt),
    });
    try {
      await validateByokModel(input);
      log.info("sidecar.byok_validation.ok", {
        model_id: input.model.id,
        phase_ms: elapsedMs(validationStartedAt),
        total_ms: elapsedMs(mainStartedAt),
      });
    } catch (err) {
      const e = err as Error;
      const message = e.message ?? String(e);
      const code = message.startsWith("byok_model_not_found")
        ? "byok_model_not_found"
        : "byok_validation_failed";
      log.error("sidecar.byok_validation.failed", {
        code,
        message,
        phase_ms: elapsedMs(validationStartedAt),
        total_ms: elapsedMs(mainStartedAt),
      });
      emit({
        type: "error",
        data: { code, message, recoverable: false },
      });
      emit({
        type: "agent_end",
        data: { finish_reason: "Error" },
      });
      return 1;
    }
  }

  try {
    const runStartedAt = Date.now();
    await runAgent(input, controlStream, mockedModel);
    log.info("sidecar.exit.ok", {
      session_id: input.session_id,
      run_ms: elapsedMs(runStartedAt),
      total_ms: elapsedMs(mainStartedAt),
    });
    return 0;
  } catch (err) {
    const e = err as Error;
    log.error("sidecar.run_failed", {
      message: e.message,
      stack: e.stack ?? "(no stack)",
      total_ms: elapsedMs(mainStartedAt),
    });
    emit({
      type: "error",
      data: {
        code: "run_failed",
        message: e.message,
        recoverable: false,
      },
    });
    emit({
      type: "agent_end",
      data: { finish_reason: "Error" },
    });
    return 1;
  }
}

/**
 * Top-level dispatcher. The sidecar binary multiplexes two modes:
 *   - Default: run one agent turn (reads SidecarInput from a temp file).
 *   - `--bootstrap-mcp-oauth <input>`: one-shot OAuth bootstrap for an
 *     MCP-backed connector (see `mcp/bootstrap.ts`).
 *
 * We detect the flag by scanning argv; the input file path is always
 * the LAST positional arg in either mode, matching `readSidecarInput`'s
 * existing contract.
 */
async function dispatch(): Promise<number> {
  if (process.argv.includes(BOOTSTRAP_MCP_OAUTH_FLAG)) {
    const inputPath = process.argv[process.argv.length - 1];
    if (!inputPath || inputPath === BOOTSTRAP_MCP_OAUTH_FLAG) {
      log.error("sidecar.bootstrap_mcp_oauth.missing_input", {
        argv: process.argv.slice(1),
      });
      return 1;
    }
    return runBootstrapMcpOauth(inputPath);
  }
  if (process.argv.includes(CLEAR_MCP_OAUTH_FLAG)) {
    const inputPath = process.argv[process.argv.length - 1];
    if (!inputPath || inputPath === CLEAR_MCP_OAUTH_FLAG) {
      log.error("sidecar.clear_mcp_oauth.missing_input", {
        argv: process.argv.slice(1),
      });
      return 1;
    }
    return runClearMcpOauth(inputPath);
  }
  return main();
}

void dispatch().then((code) => process.exit(code));
