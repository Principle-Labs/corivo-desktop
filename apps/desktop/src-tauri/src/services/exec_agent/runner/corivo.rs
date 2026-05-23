//! Drive one user turn through the bundled `corivo-agent` sidecar
//! (Bun-compiled TS) — the Phase B replacement for the legacy `claude`
//! CLI runner.
//!
//! Lifecycle (mirrors agent-sidecar-spec §5):
//!   1. Locate `corivo-agent` next to the main exe (same pattern as
//!      `corivo-mcp`).
//!   2. Generate a per-turn UDS path under `$TMPDIR` and start the
//!      [`crate::services::exec_agent::rpc_server`] on it BEFORE we
//!      spawn the child — otherwise the sidecar's first
//!      `recall_screen_history` call would race the bind.
//!   3. Spawn the sidecar with stdin closed (`Stdio::null`) and stdout
//!      piped. The §5.1 [`SidecarInput`] is handed to the child via an
//!      argv-positional temp file path — Bun's `--compile` runtime has
//!      a stdin pipe bug that hangs the binary, and writing a multi-KB
//!      payload to an unconsumed pipe was itself blocking until the
//!      sidecar exited (then surfacing as EPIPE). See CO-98.
//!   4. Pump stdout NDJSON line-by-line through
//!      [`super::super::protocol::corivo`] into the supplied
//!      `StreamEmitter` (the same wire the frontend already consumes).
//!   5. On `agent_end` (or stdout EOF) await the child, shutdown the
//!      RPC server, and return.
//!
//! Phase C reshapes auth + model selection: CorivoProxy mode (default,
//! Corivo-managed credentials + model picker) and BYOK mode
//! (user-supplied key + model id) flow through the same `CorivoAuth`
//! shape (spec §7.3 / §7.4 / §7.5).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{ChildStderr, Command};
use tokio::sync::Notify;

use crate::domain::config::{ApiShape, ThinkingLevel};
use crate::error::{CorivoError, Result};
use crate::services::cloud::CloudSessionService;
use crate::services::recall::stream::{FinishReason, StreamEmitter};

use super::super::protocol::corivo as proto;
use super::super::rpc_server::RpcDeps;
use super::CorivoAuth;

/// Compaction model defaults per spec §7.6 — used in test fixtures
/// here. The production path resolves them in
/// `commands::exec_agent::resolve_compaction_partner` (catalog lookup
/// for CorivoProxy, hardcoded haiku/4o-mini for BYOK).
#[cfg(test)]
const DEFAULT_COMPACTION_MODEL_ANTHROPIC: &str = "claude-haiku-4-5";
#[cfg(test)]
const DEFAULT_COMPACTION_MODEL_OPENAI: &str = "gpt-4o-mini";

/// Optional focus context Quick Ask hands to the sidecar. Mirrors the
/// command-layer `FocusContextPayload` but lives here so the runner
/// signature doesn't depend on the Tauri command crate.
#[derive(Debug, Default)]
pub struct FocusContextInput {
    pub frame_id: String,
    pub summary: String,
    pub primary_text: String,
    pub selection: Option<String>,
}

impl FocusContextInput {
    fn to_json(&self) -> Value {
        json!({
            "frame_id": self.frame_id,
            "summary": self.summary,
            "primary_text": self.primary_text,
            "selection": self.selection,
        })
    }
}

/// Run one full turn. The runner owns its own per-turn UDS socket
/// (see [`super::super::rpc_server`]) for the sidecar's
/// `recall_screen_history` / `ask_permission` callbacks.
/// Full user-facing tool set (memory-system-spec §10.3). Background
/// agent tasks pass a restricted subset via the `tools_native`
/// parameter on the lower-level `run_turn_with_tools`.
pub const DEFAULT_NATIVE_TOOLS: &[&str] = &[
    // memory_search supersedes the old recall_screen_history (it
    // queries frames + notes + messages in one call). We keep the old
    // tool registered for one more release for backward compatibility
    // with cached agent state — Step 3 cleanup removes it.
    "recall_screen_history",
    "memory_search",
    "ask_permission",
    "save_note",
    "thread_search",
    "chat_thread_get",
    "note_list",
    "read",
    "bash",
    "edit",
    "write",
    "grep",
    "find",
    "ls",
];

pub async fn run_turn(
    thread_id: &str,
    user_content: String,
    auth: CorivoAuth,
    thread_model_id: String,
    thread_api_shape: ApiShape,
    thinking_level: ThinkingLevel,
    compaction_model_id: String,
    extra_system_prompt: Option<String>,
    focus_context: Option<FocusContextInput>,
    sessions_dir: PathBuf,
    bundled_skills_dir: Option<PathBuf>,
    connectors_snapshot: Vec<crate::services::connector::ConnectorTokenSnapshot>,
    mcp_server_specs: Vec<Value>,
    deps: RpcDeps,
    emitter: &StreamEmitter,
    cancel_signal: Arc<Notify>,
    cloud_session: Option<Arc<dyn CloudSessionService>>,
) -> Result<()> {
    run_turn_with_tools(
        thread_id,
        user_content,
        auth,
        thread_model_id,
        thread_api_shape,
        thinking_level,
        compaction_model_id,
        extra_system_prompt,
        focus_context,
        sessions_dir,
        bundled_skills_dir,
        connectors_snapshot,
        mcp_server_specs,
        deps,
        emitter,
        cancel_signal,
        DEFAULT_NATIVE_TOOLS,
        cloud_session,
    )
    .await
}

/// Lower-level entrypoint used by both user turns and background agent
/// tasks. `tools_native` is the exact `tools.native` array the sidecar
/// receives — restrict it for background tasks to enforce the §11.3
/// whitelist.
pub async fn run_turn_with_tools(
    thread_id: &str,
    user_content: String,
    auth: CorivoAuth,
    thread_model_id: String,
    thread_api_shape: ApiShape,
    thinking_level: ThinkingLevel,
    compaction_model_id: String,
    extra_system_prompt: Option<String>,
    focus_context: Option<FocusContextInput>,
    sessions_dir: PathBuf,
    // App-private bundled skills root. Resolved by the command layer
    // from `<resource_dir>/bundled-skills/` (dev fallback to repo
    // source). `None` when the resource isn't shipped — the sidecar
    // falls back to user-only skill sources.
    bundled_skills_dir: Option<PathBuf>,
    connectors_snapshot: Vec<crate::services::connector::ConnectorTokenSnapshot>,
    // mcp_server_specs: one JSON object per enabled `mcpServer`-shape
    // connector (`name`, `transport`, `url`, `token_cache_dir`). Goes
    // straight into `input.tools.mcp_servers`; the sidecar's `mcporter`
    // runtime reconnects against the cache directory.
    mcp_server_specs: Vec<Value>,
    deps: RpcDeps,
    emitter: &StreamEmitter,
    // `notify_one()` from `exec_agent_cancel` pre-empts the stdout
    // pump, kills the child, and finishes the turn with
    // `FinishReason::Cancelled`. The same handle is registered in
    // `AppState::pending_turns` keyed by `thread_id` so the cancel
    // command can reach it.
    cancel_signal: Arc<Notify>,
    tools_native: &[&str],
    // `Some` in CorivoProxy mode. When the sidecar emits an
    // `auth_failed` upstream error (gateway returned 401 on the
    // cached api_key — typical cause: cloud session reaped on the
    // server, gateway key rotated), we fire a best-effort
    // cloud-session refresh so the next turn picks up fresh
    // (apiHost, apiKey) without the user having to re-login. If the
    // refresh itself fails, `with_auth_retry` inside
    // `refresh_agent_creds_now` routes to logout-required handling and emits the
    // standard `corivo:auth_required` event.
    cloud_session: Option<Arc<dyn CloudSessionService>>,
) -> Result<()> {
    let runner_started_at = Instant::now();
    if !auth.has_credentials() {
        return Err(CorivoError::Internal(
            "执行 Agent 设为 Corivo 模式,但没有可用凭据 — \
             请到设置页保存 BYOK key 或登录 Corivo 获得会话凭据"
                .to_string(),
        ));
    }
    if thread_model_id.trim().is_empty() || thread_model_id == "claude:legacy" {
        return Err(CorivoError::Internal(format!(
            "thread {thread_id} 没有有效的 bound_model_id — 请在新会话前确认设置页已选择模型"
        )));
    }

    tracing::info!(
        target: "exec_agent",
        thread_id,
        model_id = %thread_model_id,
        api_shape = ?thread_api_shape,
        thinking_level = thinking_level.as_str(),
        user_message_chars = user_content.chars().count(),
        native_tools_count = tools_native.len(),
        connectors_count = connectors_snapshot.len(),
        mcp_servers_count = mcp_server_specs.len(),
        has_focus_context = focus_context.is_some(),
        has_extra_system_prompt = extra_system_prompt
            .as_ref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false),
        "exec_agent.corivo.prepare_start"
    );

    // Spec §8.1: ensure the jsonl session directory exists before we hand
    // its path to the sidecar. SessionManager would `mkdirSync(recursive)`
    // anyway, but having Rust own dir creation keeps permission failures
    // surfacing as a clean Tauri error rather than as a sidecar crash.
    let sessions_dir_started_at = Instant::now();
    if let Err(e) = std::fs::create_dir_all(&sessions_dir) {
        return Err(CorivoError::Internal(format!(
            "exec_agent.sessions_dir.mkdir failed ({}): {e}",
            sessions_dir.display()
        )));
    }
    tracing::info!(
        target: "exec_agent",
        thread_id,
        phase_ms = elapsed_ms(sessions_dir_started_at),
        total_ms = elapsed_ms(runner_started_at),
        sessions_dir = %sessions_dir.display(),
        "exec_agent.corivo.sessions_dir_ready"
    );

    let locate_started_at = Instant::now();
    let launch = locate_sidecar()?;
    tracing::info!(
        target: "exec_agent",
        thread_id,
        phase_ms = elapsed_ms(locate_started_at),
        total_ms = elapsed_ms(runner_started_at),
        sidecar = %launch.trace_label(),
        "exec_agent.corivo.sidecar_located"
    );
    let socket_path = make_rpc_socket_path();
    // Best-effort cleanup of a stale socket from a prior crash.
    let _ = std::fs::remove_file(&socket_path);

    // Bind the RPC server BEFORE the child spawns; otherwise the sidecar
    // could connect and fire its first request before we're listening.
    let rpc_started_at = Instant::now();
    let rpc_handle = super::super::rpc_server::start(&socket_path, deps).await?;
    tracing::info!(
        target: "exec_agent",
        thread_id,
        phase_ms = elapsed_ms(rpc_started_at),
        total_ms = elapsed_ms(runner_started_at),
        rpc_socket = %socket_path.display(),
        "exec_agent.corivo.rpc_ready"
    );

    let input_started_at = Instant::now();
    let input = build_sidecar_input(
        thread_id,
        &user_content,
        &auth,
        &thread_model_id,
        thread_api_shape,
        thinking_level,
        &compaction_model_id,
        extra_system_prompt.as_deref(),
        focus_context.as_ref(),
        &sessions_dir,
        bundled_skills_dir.as_deref(),
        &connectors_snapshot,
        &mcp_server_specs,
        &socket_path,
        tools_native,
    );
    let payload = format!("{}\n", input);
    tracing::info!(
        target: "exec_agent",
        thread_id,
        phase_ms = elapsed_ms(input_started_at),
        total_ms = elapsed_ms(runner_started_at),
        input_bytes = payload.len(),
        "exec_agent.corivo.input_built"
    );

    // Write the §5.1 SidecarInput to a temp file and pass the path as
    // argv[1]. The sidecar reads from this file, NOT stdin — Bun
    // --compile 1.3 has a stdin pipe bug that hangs the binary. See
    // packages/agent/src/main.ts for the empirical findings.
    //
    // Filename includes a nanosecond timestamp + pid so concurrent turns
    // don't clobber each other. The sidecar is responsible for nothing;
    // we (the parent) clean up after the child exits.
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let input_path = std::env::temp_dir().join(format!("corivo-agent-input-{pid}-{nanos}.json"));
    let input_write_started_at = Instant::now();
    std::fs::write(&input_path, payload.as_bytes()).map_err(|e| {
        CorivoError::Internal(format!(
            "exec_agent.corivo.write_input failed ({}): {e}",
            input_path.display()
        ))
    })?;
    tracing::info!(
        target: "exec_agent",
        thread_id,
        phase_ms = elapsed_ms(input_write_started_at),
        total_ms = elapsed_ms(runner_started_at),
        input_path = %input_path.display(),
        input_bytes = payload.len(),
        "exec_agent.corivo.input_written"
    );

    let mut cmd = launch.command_with_args([input_path.as_os_str()]);
    // stdin is closed — the sidecar reads its input from the temp file
    // at argv[1] and never touches stdin (Bun --compile has a stdin
    // hang bug; see packages/agent/src/main.ts::readSidecarInput).
    // Piping stdin AND writing the payload caused CO-98: the JSON often
    // exceeds the macOS pipe buffer (~16KB), so `write_all` blocked
    // until the child eventually exited and the parent saw EPIPE.
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // The sidecar handles auth itself; it pulls model id + key from the
    // SidecarInput JSON. Wipe any inherited Anthropic env so a stale
    // base url / token can't override the explicit input.
    cmd.env_remove("ANTHROPIC_API_KEY");
    cmd.env_remove("ANTHROPIC_AUTH_TOKEN");
    cmd.env_remove("ANTHROPIC_BASE_URL");

    tracing::info!(
        sidecar = %launch.trace_label(),
        rpc_socket = %socket_path.display(),
        thread_id,
        "exec_agent.spawn_corivo"
    );

    let spawn_started_at = Instant::now();
    let spawn_result = cmd.spawn();
    let mut child = match spawn_result {
        Ok(c) => c,
        Err(e) => {
            // Tear down the listener so the next turn's bind doesn't
            // fail on EADDRINUSE. `shutdown_blocking` consumes the
            // handle, so we can't fall through past this branch.
            rpc_handle.shutdown_blocking();
            // PATH is recovered from the user's login shell at boot
            // (`shell_path::resolve_for_child`), so spawn-by-name still
            // sees Homebrew / asdf / cargo / bun in both `app:dev`
            // AND Finder-launched packaged builds. A failure here means
            // the binary genuinely isn't on the user's machine (BunRun)
            // or the bundled sidecar is missing (Compiled).
            let hint = match &launch {
                SidecarLaunch::BunRun { .. } => {
                    "install `bun` (https://bun.sh) — \
                    debug builds of the desktop spawn the sidecar via `bun run`"
                }
                SidecarLaunch::Compiled(_) => {
                    "the bundled corivo-agent binary is \
                    missing or not executable — re-run `pnpm app:build`"
                }
            };
            return Err(CorivoError::Internal(format!(
                "exec_agent.corivo.spawn failed ({} after {}ms): {e} — {hint}",
                launch.trace_label(),
                elapsed_ms(spawn_started_at)
            )));
        }
    };
    let child_started_at = Instant::now();

    tracing::info!(
        child_pid = ?child.id(),
        thread_id,
        spawn_ms = elapsed_ms(spawn_started_at),
        total_ms = elapsed_ms(runner_started_at),
        "exec_agent.corivo_started"
    );

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| CorivoError::Internal("corivo-agent stdout pipe missing".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| CorivoError::Internal("corivo-agent stderr pipe missing".to_string()))?;

    // Spawn the stderr drain BEFORE any other potentially-fallible work
    // below — if the sidecar exits early (bad input file, missing bun,
    // panic in bootstrap-pi-env), this is the only path that gets its
    // panic message into the daily log. Pre-CO-98 we spawned this AFTER
    // a blocking stdin write, which meant a sidecar that died during
    // startup left zero log breadcrumbs.
    tokio::spawn(drain_stderr(stderr, runner_started_at, child_started_at));
    // §5.3 control channel (cancel/steer/compact) is not wired; when it
    // lands it will go over the existing UDS RPC socket, NOT stdin.
    // Reasoning: the sidecar's `readSidecarInput` deliberately avoids
    // stdin because Bun's `--compile` runtime hangs forever reading it
    // (reproduced on bun 1.3.11 darwin-arm64/-x64). Keeping stdin closed
    // here also removes the EPIPE failure mode in CO-98.

    let mut lines = BufReader::new(stdout).lines();
    let mut finish_reason: Option<FinishReason> = None;
    let mut events_count: u64 = 0;
    let mut first_event_logged = false;
    let mut first_text_delta_logged = false;
    loop {
        // Race the next stdout line against an external cancel signal.
        // `biased` makes the cancel branch checked first so a signal
        // that fires while a line is also ready resolves as Cancelled
        // rather than EndTurn (matters when the user hits Stop right as
        // the model emits `agent_end`). `Notify::notified()` registers
        // a fresh waiter each iteration; permits left over from a
        // previous tick are still honored.
        let line_opt = tokio::select! {
            biased;
            _ = cancel_signal.notified() => {
                tracing::info!(thread_id, "exec_agent.corivo.cancel_received");
                // SIGKILL via tokio::process::Child::kill — the sidecar
                // is single-shot per turn so we don't need a graceful
                // shutdown handshake. `kill_on_drop(true)` would also
                // catch this on scope exit, but issuing it explicitly
                // here lets the child clean up its UDS socket / temp
                // files before we tear down the rpc server.
                if let Err(e) = child.kill().await {
                    tracing::warn!(error = %e, "exec_agent.corivo.cancel_kill_failed");
                }
                finish_reason = Some(FinishReason::Cancelled);
                break;
            }
            line_res = lines.next_line() => {
                line_res.map_err(|e| {
                    CorivoError::Internal(format!("corivo-agent stdout read failed: {e}"))
                })?
            }
        };
        let Some(line) = line_opt else {
            break;
        };
        match proto::parse_line(&line) {
            Ok(Some(envelope)) => {
                events_count += 1;
                if !first_event_logged {
                    first_event_logged = true;
                    tracing::info!(
                        target: "exec_agent",
                        thread_id,
                        event_type = %envelope.event_type,
                        ms_since_spawn = elapsed_ms(child_started_at),
                        total_ms = elapsed_ms(runner_started_at),
                        "exec_agent.corivo.first_stdout_event"
                    );
                }
                if envelope.event_type == "text_delta" && !first_text_delta_logged {
                    first_text_delta_logged = true;
                    tracing::info!(
                        target: "exec_agent",
                        thread_id,
                        ms_since_spawn = elapsed_ms(child_started_at),
                        total_ms = elapsed_ms(runner_started_at),
                        "exec_agent.corivo.first_text_delta"
                    );
                }
                // text_delta fires hundreds of times per turn — keep that
                // at TRACE so the default `debug` filter doesn't drown.
                // Everything else (tool calls, turn boundaries, errors)
                // is rare enough to be useful at DEBUG.
                if envelope.event_type == "text_delta" || envelope.event_type == "thinking_delta" {
                    tracing::trace!(
                        target: "exec_agent",
                        event_type = %envelope.event_type,
                        "exec_agent.corivo.event"
                    );
                } else {
                    tracing::debug!(
                        target: "exec_agent",
                        event_type = %envelope.event_type,
                        "exec_agent.corivo.event"
                    );
                }
                // Sidecar tags upstream 401s with `code: "auth_failed"`
                // on the `error` event. Hijack the dispatch here:
                //   1. Fire a best-effort cloud-session refresh so
                //      the next turn carries fresh creds.
                //   2. Tell the UI the turn ended and the user should
                //      re-send (Chinese, since UI strings are 中文).
                //   3. Mark the turn `Error` and break the pump — the
                //      sidecar will follow with `agent_end` but we've
                //      already short-circuited.
                if envelope.event_type == "error"
                    && envelope.data.get("code").and_then(Value::as_str) == Some("auth_failed")
                {
                    tracing::warn!(
                        target: "exec_agent",
                        thread_id,
                        "exec_agent.corivo.auth_failed_from_sidecar"
                    );
                    if let Some(session) = cloud_session.as_ref().cloned() {
                        tauri::async_runtime::spawn(async move {
                            if let Err(error) = session.refresh_agent_creds_now().await {
                                tracing::warn!(
                                    error = %error,
                                    "exec_agent.corivo.auth_failed_refresh_failed"
                                );
                            }
                        });
                    }
                    let detail = envelope
                        .data
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let user_msg = if detail.is_empty() {
                        "会话凭证已过期,凭证已自动刷新,请重新发送".to_string()
                    } else {
                        format!("会话凭证已过期,凭证已自动刷新,请重新发送 — {detail}")
                    };
                    let _ = emitter.error(&user_msg);
                    finish_reason = Some(FinishReason::Error);
                    break;
                }
                match proto::forward(envelope, emitter) {
                    Ok(Some(reason)) => {
                        finish_reason = Some(reason);
                        break;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::warn!(error = %e, "exec_agent.corivo.forward_failed");
                        let _ = emitter.error(&e.to_string());
                        finish_reason = Some(FinishReason::Error);
                        break;
                    }
                }
            }
            Ok(None) => {}
            // Don't abort the turn on a single bad line — keep draining,
            // just like the claude path. Most likely cause is a future
            // event variant we don't model yet.
            Err(e) => {
                tracing::warn!(error = %e, raw = %line, "exec_agent.corivo.parse_line_skipped")
            }
        }
    }

    // Same EOF semantics as the claude runner: an EOF without a
    // terminating event = sidecar crashed / network died / model errored
    // before flushing `agent_end`. Surface that as Error rather than the
    // historical silent EndTurn.
    let final_reason = if let Some(r) = finish_reason {
        let wait_started_at = Instant::now();
        match child.wait().await {
            Ok(status) => tracing::info!(
                target: "exec_agent",
                thread_id,
                status = ?status,
                wait_ms = elapsed_ms(wait_started_at),
                total_ms = elapsed_ms(runner_started_at),
                "exec_agent.corivo.child_wait_done"
            ),
            Err(error) => tracing::warn!(
                target: "exec_agent",
                thread_id,
                error = %error,
                wait_ms = elapsed_ms(wait_started_at),
                total_ms = elapsed_ms(runner_started_at),
                "exec_agent.corivo.child_wait_failed"
            ),
        }
        r
    } else {
        let wait_started_at = Instant::now();
        let detail = match child.wait().await {
            Ok(status) if status.success() => {
                "corivo-agent exited cleanly without emitting agent_end".to_string()
            }
            Ok(status) => format!("corivo-agent exited with {status} before finishing the turn"),
            Err(e) => format!("corivo-agent wait failed: {e}"),
        };
        tracing::warn!(
            target: "exec_agent",
            thread_id,
            detail = %detail,
            wait_ms = elapsed_ms(wait_started_at),
            total_ms = elapsed_ms(runner_started_at),
            "exec_agent.corivo.stdout_eof_without_finish"
        );
        let _ = emitter.error(&detail);
        FinishReason::Error
    };

    let shutdown_started_at = Instant::now();
    rpc_handle.shutdown().await;
    tracing::debug!(
        target: "exec_agent",
        thread_id,
        phase_ms = elapsed_ms(shutdown_started_at),
        total_ms = elapsed_ms(runner_started_at),
        "exec_agent.corivo.rpc_shutdown_done"
    );
    let _ = emitter.finish(final_reason);
    // Best-effort cleanup of the input temp file. If it fails (e.g. user
    // already swept /tmp) we don't care — only logs at trace level.
    if let Err(e) = std::fs::remove_file(&input_path) {
        tracing::trace!(error = %e, path = %input_path.display(), "exec_agent.corivo.input_cleanup_skipped");
    }
    tracing::info!(
        target: "exec_agent",
        thread_id,
        final_reason = ?final_reason,
        events_count,
        saw_first_stdout_event = first_event_logged,
        saw_first_text_delta = first_text_delta_logged,
        total_ms = elapsed_ms(runner_started_at),
        "exec_agent.corivo.turn_done"
    );
    Ok(())
}

/// Drain the corivo-agent sidecar's stderr and forward each line to
/// `tracing` with the appropriate level.
///
/// The sidecar uses `packages/agent/src/log.ts`, which writes lines in the
/// shape `[<level>] <msg> <fields_json?>`. Parsing the level prefix lets
/// us preserve the sidecar's own log levels (debug/info/warn/error) all
/// the way into the daily log file rather than collapsing everything to
/// `warn`. Lines without a recognised prefix fall back to `warn` (so a
/// raw runtime panic from Bun still surfaces loudly).
async fn drain_stderr(stderr: ChildStderr, runner_started_at: Instant, child_started_at: Instant) {
    tracing::info!(
        target: "exec_agent",
        total_ms = elapsed_ms(runner_started_at),
        "exec_agent.corivo.drain_stderr.started"
    );
    let mut lines = BufReader::new(stderr).lines();
    let mut first_line_logged = false;
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        if !first_line_logged {
            first_line_logged = true;
            tracing::info!(
                target: "exec_agent",
                ms_since_spawn = elapsed_ms(child_started_at),
                total_ms = elapsed_ms(runner_started_at),
                "exec_agent.corivo.first_stderr"
            );
        }
        match parse_level_prefix(&line) {
            Some(("trace", rest)) => {
                tracing::trace!(target: "corivo_agent", message = %rest)
            }
            Some(("debug", rest)) => {
                tracing::debug!(target: "corivo_agent", message = %rest)
            }
            Some(("info", rest)) => {
                tracing::info!(target: "corivo_agent", message = %rest)
            }
            Some(("warn", rest)) => {
                tracing::warn!(target: "corivo_agent", message = %rest)
            }
            Some(("error", rest)) => {
                tracing::error!(target: "corivo_agent", message = %rest)
            }
            _ => tracing::warn!(target: "corivo_agent", raw = %line),
        }
    }
    tracing::info!(
        target: "exec_agent",
        total_ms = elapsed_ms(runner_started_at),
        saw_line = first_line_logged,
        "exec_agent.corivo.drain_stderr.eof"
    );
}

/// Parse a `[<level>] <rest>` prefix. Returns `Some((level, rest))` on
/// match, `None` otherwise. Cheap byte-level check, no regex.
fn parse_level_prefix(line: &str) -> Option<(&'static str, &str)> {
    let rest = line.strip_prefix('[')?;
    let close = rest.find(']')?;
    let level = &rest[..close];
    let after = rest[close + 1..].trim_start();
    let canonical = match level {
        "trace" => "trace",
        "debug" => "debug",
        "info" => "info",
        "warn" => "warn",
        "error" => "error",
        _ => return None,
    };
    Some((canonical, after))
}

fn elapsed_ms(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn build_sidecar_input(
    thread_id: &str,
    user_content: &str,
    auth: &CorivoAuth,
    thread_model_id: &str,
    thread_api_shape: ApiShape,
    thinking_level: ThinkingLevel,
    compaction_model_id: &str,
    extra_system_prompt: Option<&str>,
    focus_context: Option<&FocusContextInput>,
    sessions_dir: &Path,
    bundled_skills_dir: Option<&Path>,
    connectors_snapshot: &[crate::services::connector::ConnectorTokenSnapshot],
    mcp_server_specs: &[Value],
    rpc_socket: &Path,
    tools_native: &[&str],
) -> Value {
    let api_shape_str = thread_api_shape.as_str();
    let mut input = json!({
        // Spec §8.1: corivo-agent uses thread_id directly as the jsonl
        // session filename. No UUID v5 mapping anymore (that was a
        // claude-CLI workaround).
        "session_id": thread_id,
        "user_message": {
            "role": "user",
            "content": user_content,
        },
        // Spec §8.2: pull from the thread's permanent binding rather
        // than the live config snapshot.
        "model": {
            "id": thread_model_id,
            "api_shape": api_shape_str,
            "thinking_level": thinking_level.as_str(),
        },
        "compaction_model": {
            "id": compaction_model_id,
            "api_shape": api_shape_str,
        },
        "auth": auth.to_json(),
        "tools": {
            // Native AgentTools the sidecar registers (see
            // packages/agent/src/native-tools/index.ts):
            //   - corivo callbacks (recall_screen_history, ask_permission)
            //     route through the per-turn UDS RPC server.
            //   - read/bash/edit/write/grep/find/ls come from
            //     pi-coding-agent's `createCodingTools()` and run in-
            //     process against the user's home directory. Bash
            //     dangerous-command gating via `ask_permission` is
            //     spec'd (§6.4) but not yet wired — the model has
            //     unrestricted shell today, gate before shipping.
            // memory-system-spec §11.3: the caller decides exactly which
            // tools the sidecar registers — DEFAULT_NATIVE_TOOLS for
            // user-facing turns, a per-task whitelist for background
            // agent tasks.
            "native": tools_native,
            // `mcpServer`-shape connectors that are enabled + bootstrapped.
            // Empty until at least one MCP-backed connector (e.g. Linear)
            // has been installed via `connector_install_mcp`. Each entry
            // matches `packages/agent/src/types.ts::McpServerSpec`.
            "mcp_servers": mcp_server_specs,
        },
        "rpc_socket": rpc_socket.to_string_lossy(),
        // Spec §8.1: jsonl session store root. The sidecar opens
        // `${sessions_dir}/${session_id}.jsonl`, seeds Agent state from
        // it on boot, and appends new messages + (when triggered) one
        // compaction summary entry per turn. Rust owns dir creation
        // (just above) so permission failures surface here.
        "sessions_dir": sessions_dir.to_string_lossy(),
    });
    if let Some(prompt) = extra_system_prompt {
        if !prompt.trim().is_empty() {
            input["system_prompt_extra"] = json!(prompt);
        }
    }
    if let Some(focus) = focus_context {
        input["focus_context"] = focus.to_json();
    }
    if let Some(dir) = bundled_skills_dir {
        input["bundled_skills_dir"] = json!(dir.to_string_lossy());
    }
    // Per-turn connector snapshot. Only emit the `connectors` block when
    // the user has at least one connector enabled — older agent builds
    // that predate connector support won't choke on an unrecognized
    // top-level key but the symmetric "snapshot is omitted means no
    // connectors" semantic is cleaner.
    if !connectors_snapshot.is_empty() {
        input["connectors"] = json!({
            "enabled": connectors_snapshot
                .iter()
                .map(|s| json!({
                    "id": s.id,
                    "account_email": s.account_email,
                    "granted_scopes": s.granted_scopes,
                    "access_token": s.access_token,
                    "expires_at": s.expires_at,
                }))
                .collect::<Vec<_>>(),
        });
    }
    input
}

/// How the sidecar is launched for this turn.
///
/// Two delivery shapes are supported because dev iteration on the
/// Bun-compiled binary is painful (every TS edit means rebuild + lipo
/// + atomic file replace, ~6 seconds, AND macOS holds the previous
/// Mach-O via the codesign cache — see the "hang" diagnostic in the
/// May 2026 thread). The dev path forwards to `bun run` against the
/// live source tree so a code edit lands on the very next `chat_send`.
#[derive(Debug)]
pub(crate) enum SidecarLaunch {
    /// Production / release path: `binaries/corivo-agent-<triple>` (or,
    /// in the bundled `.app`, `Contents/MacOS/corivo-agent`).
    Compiled(PathBuf),
    /// Dev path: spawn a system `bun` against the live monorepo source.
    /// Cost: ~200–400 ms extra startup vs. compiled. Benefit: every
    /// turn re-reads the latest TS so editing
    /// `packages/agent/src/*.ts` hot-applies on the next message.
    BunRun {
        /// Absolute path to `packages/agent/src/main.ts`.
        source: PathBuf,
    },
}

impl SidecarLaunch {
    /// Short label for tracing (`compiled:/path/...` vs
    /// `bun-run:/path/...`).
    pub(crate) fn trace_label(&self) -> String {
        match self {
            Self::Compiled(p) => format!("compiled:{}", p.display()),
            Self::BunRun { source } => format!("bun-run:{}", source.display()),
        }
    }

    /// Build a `tokio::process::Command` that invokes the sidecar with
    /// the supplied positional args (whatever they are — input file
    /// path for normal mode, or `--bootstrap-mcp-oauth <file>` for the
    /// MCP OAuth one-shot). Does not configure stdin/stdout/stderr —
    /// the caller decides.
    pub(crate) fn command_with_args<I, S>(&self, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        match self {
            Self::Compiled(bin) => {
                let mut c = Command::new(bin);
                for a in args {
                    c.arg(a);
                }
                c
            }
            Self::BunRun { source } => {
                let mut c = Command::new("bun");
                c.arg("run").arg(source);
                for a in args {
                    c.arg(a);
                }
                c
            }
        }
    }
}

/// Resolve the sidecar launch shape for this run. Order:
///   1. `CORIVO_AGENT_FORCE_COMPILED=1` → always compiled (escape
///      hatch for testing the prod binary path from a debug build).
///   2. `CORIVO_AGENT_DEV_SOURCE=<path>` → bun-run that explicit
///      source file (CI / unusual layouts).
///   3. Debug build of the desktop crate AND we can locate
///      `packages/agent/src/main.ts` by walking up from `current_exe`
///      → bun-run.
///   4. Otherwise → compiled binary next to `current_exe`.
pub(crate) fn locate_sidecar() -> Result<SidecarLaunch> {
    if std::env::var_os("CORIVO_AGENT_FORCE_COMPILED").is_some() {
        return locate_compiled().map(SidecarLaunch::Compiled);
    }

    if let Some(path) = std::env::var_os("CORIVO_AGENT_DEV_SOURCE") {
        let source = PathBuf::from(path);
        if !source.is_file() {
            return Err(CorivoError::Internal(format!(
                "CORIVO_AGENT_DEV_SOURCE points at {} which doesn't exist",
                source.display()
            )));
        }
        return Ok(SidecarLaunch::BunRun { source });
    }

    if cfg!(debug_assertions) {
        if let Some(source) = locate_dev_source_in_monorepo()? {
            return Ok(SidecarLaunch::BunRun { source });
        }
    }

    locate_compiled().map(SidecarLaunch::Compiled)
}

/// Walk up from `current_exe` looking for the monorepo source path
/// `packages/agent/src/main.ts`. Returns `None` (not Err) when the
/// debug binary is running outside the monorepo (e.g. a CI artifact
/// run from /tmp) — caller falls back to the compiled binary.
fn locate_dev_source_in_monorepo() -> Result<Option<PathBuf>> {
    let me = std::env::current_exe()
        .map_err(|e| CorivoError::Internal(format!("current_exe failed: {e}")))?;
    let mut cur: Option<&Path> = me.parent();
    while let Some(dir) = cur {
        let candidate = dir
            .join("packages")
            .join("agent")
            .join("src")
            .join("main.ts");
        if candidate.is_file() {
            return Ok(Some(candidate));
        }
        cur = dir.parent();
    }
    Ok(None)
}

/// Locate the compiled `corivo-agent` binary next to the main exe (the
/// layout Tauri's `externalBin` produces in both `target/<profile>/`
/// and the bundled `.app/Contents/MacOS/` or `Corivo/`).
///
/// macOS / Linux: bare `corivo-agent` (Mach-O / ELF).
/// Windows: `corivo-agent.exe`.
fn locate_compiled() -> Result<PathBuf> {
    let me = std::env::current_exe()
        .map_err(|e| CorivoError::Internal(format!("current_exe failed: {e}")))?;
    let dir = me
        .parent()
        .ok_or_else(|| CorivoError::Internal("current_exe has no parent directory".to_string()))?;
    let filename = if cfg!(windows) {
        "corivo-agent.exe"
    } else {
        "corivo-agent"
    };
    let sidecar = dir.join(filename);
    if !sidecar.is_file() {
        return Err(CorivoError::Internal(format!(
            "corivo-agent sidecar not found at {} — run `pnpm --filter @corivo/agent build`",
            sidecar.display()
        )));
    }
    Ok(sidecar)
}

fn make_rpc_socket_path() -> PathBuf {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    // Windows: named pipes live under \\.\pipe\<name>, not the filesystem.
    // tokio's NamedPipeServer::create requires this prefix, and Node's
    // `net.connect(path)` transparently routes paths beginning with
    // `\\.\pipe\` to the named-pipe namespace — so the agent's rpc.ts
    // works unchanged.
    #[cfg(windows)]
    {
        return PathBuf::from(format!(r"\\.\pipe\corivo-agent-rpc-{pid}-{nanos}"));
    }
    #[cfg(not(windows))]
    {
        std::env::temp_dir().join(format!("corivo-agent-rpc-{pid}-{nanos}.sock"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn byok(api_key: &str) -> CorivoAuth {
        CorivoAuth::Byok {
            base_url: None,
            api_key: api_key.to_string(),
        }
    }

    fn sessions() -> PathBuf {
        PathBuf::from("/tmp/corivo-agent-sessions")
    }

    #[test]
    fn build_sidecar_input_contains_required_fields() {
        let socket = PathBuf::from("/tmp/foo.sock");
        let sessions = sessions();
        let bundled_skills = PathBuf::from("/tmp/bundled-skills");
        let input = build_sidecar_input(
            "01ABCD",
            "hello",
            &byok("sk-ant-test"),
            "claude-sonnet-4-5-20250929",
            ApiShape::Anthropic,
            ThinkingLevel::XHigh,
            DEFAULT_COMPACTION_MODEL_ANTHROPIC,
            Some("extra prompt"),
            Some(&FocusContextInput {
                frame_id: "F1".into(),
                summary: "summary".into(),
                primary_text: "screen text".into(),
                selection: Some("highlighted".into()),
            }),
            &sessions,
            Some(&bundled_skills),
            &[],
            &[],
            &socket,
            DEFAULT_NATIVE_TOOLS,
        );
        assert_eq!(input["session_id"], "01ABCD");
        assert_eq!(input["user_message"]["content"], "hello");
        assert_eq!(input["model"]["api_shape"], "anthropic");
        assert_eq!(input["model"]["id"], "claude-sonnet-4-5-20250929");
        assert_eq!(input["model"]["thinking_level"], "xhigh");
        assert_eq!(
            input["compaction_model"]["id"],
            DEFAULT_COMPACTION_MODEL_ANTHROPIC
        );
        assert_eq!(input["auth"]["mode"], "byok");
        assert_eq!(input["auth"]["token"], "sk-ant-test");
        assert!(input["auth"]["base_url"].is_null());
        assert_eq!(input["rpc_socket"], "/tmp/foo.sock");
        assert_eq!(input["sessions_dir"], "/tmp/corivo-agent-sessions");
        assert_eq!(input["bundled_skills_dir"], "/tmp/bundled-skills");
        assert_eq!(input["system_prompt_extra"], "extra prompt");
        assert_eq!(input["focus_context"]["frame_id"], "F1");
        assert_eq!(input["focus_context"]["selection"], "highlighted");
        let native = input["tools"]["native"].as_array().unwrap();
        assert!(native
            .iter()
            .any(|v| v.as_str() == Some("recall_screen_history")));
        assert!(native.iter().any(|v| v.as_str() == Some("ask_permission")));
    }

    #[test]
    fn build_sidecar_input_omits_empty_extra_prompt() {
        let socket = PathBuf::from("/tmp/foo.sock");
        let sessions = sessions();
        let input = build_sidecar_input(
            "01ABCD",
            "hi",
            &byok("sk-ant"),
            "claude-sonnet-4-5-20250929",
            ApiShape::Anthropic,
            ThinkingLevel::Medium,
            DEFAULT_COMPACTION_MODEL_ANTHROPIC,
            Some("   "),
            None,
            &sessions,
            None,
            &[],
            &[],
            &socket,
            DEFAULT_NATIVE_TOOLS,
        );
        assert!(input.get("system_prompt_extra").is_none());
        assert!(input.get("focus_context").is_none());
        // bundled_skills_dir is omitted when the resource isn't on
        // disk (mock builds, broken installs, etc.). The sidecar then
        // falls back to user-only skill sources.
        assert!(input.get("bundled_skills_dir").is_none());
        // sessions_dir always rides along — sidecar can't fall back to a
        // sane default since it's a Bun-compiled binary with no AppData
        // knowledge of its own.
        assert_eq!(input["sessions_dir"], "/tmp/corivo-agent-sessions");
    }

    #[test]
    fn build_sidecar_input_routes_openai_compaction() {
        let socket = PathBuf::from("/tmp/foo.sock");
        let sessions = sessions();
        let input = build_sidecar_input(
            "01ABCD",
            "hi",
            &byok("sk-test"),
            "gpt-4o",
            ApiShape::Openai,
            ThinkingLevel::Medium,
            DEFAULT_COMPACTION_MODEL_OPENAI,
            None,
            None,
            &sessions,
            None,
            &[],
            &[],
            &socket,
            DEFAULT_NATIVE_TOOLS,
        );
        assert_eq!(input["model"]["api_shape"], "openai");
        assert_eq!(input["model"]["id"], "gpt-4o");
        assert_eq!(input["compaction_model"]["api_shape"], "openai");
        assert_eq!(
            input["compaction_model"]["id"],
            DEFAULT_COMPACTION_MODEL_OPENAI
        );
    }

    #[test]
    fn build_sidecar_input_corivo_proxy_carries_gateway_url() {
        let socket = PathBuf::from("/tmp/foo.sock");
        let sessions = sessions();
        // Note: `api_key` here represents the upstream provider key the
        // corivo backend hands the desktop at login (NOT the corivo
        // session token). It travels into auth.token where pi-ai uses it
        // as the upstream Anthropic/OpenAI auth header.
        let auth = CorivoAuth::CorivoProxy {
            gateway_url: "https://gateway.example.com/v1".into(),
            api_key: "sk-upstream-key".into(),
        };
        let input = build_sidecar_input(
            "01ABCD",
            "hi",
            &auth,
            "claude-sonnet-4-6",
            ApiShape::Anthropic,
            ThinkingLevel::Medium,
            "claude-haiku-4-5",
            None,
            None,
            &sessions,
            None,
            &[],
            &[],
            &socket,
            DEFAULT_NATIVE_TOOLS,
        );
        assert_eq!(input["auth"]["mode"], "corivo_proxy");
        assert_eq!(input["auth"]["base_url"], "https://gateway.example.com/v1");
        assert_eq!(input["auth"]["token"], "sk-upstream-key");
        assert_eq!(input["compaction_model"]["id"], "claude-haiku-4-5");
    }
}
