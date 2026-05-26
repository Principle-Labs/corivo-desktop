//! Spawn one background agent task end-to-end (memory-system-spec §11.4).
//!
//! Reuses `services::exec_agent::runner::corivo::run_turn_with_tools`
//! with three deltas vs. a user turn:
//!
//! * `tools.native` is the task's `tool_whitelist()` — every write
//!   tool (`save_note`, etc.) is implicitly absent.
//! * The stream channel is a discarding `tauri::ipc::Channel`; no UI
//!   subscriber. `text_delta` events are still parsed into a buffer so
//!   `consume_output` can act on the final assistant text.
//! * The kind='system' chat_thread is created fresh per task run so
//!   the diagnostic panel can drill into the full conversation +
//!   tool-call trace.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tauri::ipc::Channel;
use tokio::sync::Notify;

/// Hard wall-clock cap on one background-agent sidecar turn.
///
/// `BackgroundAgentScheduler` is single-flight FIFO, so a hung sidecar
/// (LLM stall, network drop, runaway agent loop) freezes every other
/// task behind it indefinitely. After this duration the watchdog aborts
/// the runner task, which drops the `Child` and `kill_on_drop(true)`
/// SIGKILLs the sidecar. The run is recorded as a failure and the next
/// task in the queue proceeds.
///
/// 5 minutes is generous for normal agent turns (most finish in
/// <60 s) but short enough that one bad task only delays the queue by
/// ~5 minutes instead of forever.
const BACKGROUND_TASK_TIMEOUT: Duration = Duration::from_secs(300);

use super::log::{render_sidecar_event, TaskLogger};
use super::{BackgroundAgentTask, TaskDeps};
use crate::db::repos::chat::{FinalizeAssistant, NewChatThread, NewUserMessage};
use crate::domain::chat::{ChatThreadKind, ContentBlock, MessageStatus};
use crate::error::Result;
use crate::services::exec_agent::rpc_server::RpcDeps;
use crate::services::exec_agent::runner::corivo::run_turn_with_tools;
use crate::services::exec_agent::CorivoAuth;
use crate::services::recall::stream::StreamEmitter;

/// Final state returned by the runner. `consume_output` reads it to
/// decide whether to commit side effects or leave the checkpoint
/// untouched.
pub struct TaskOutcome {
    pub thread_id: String,
    /// True when the sidecar finished cleanly. False on cancellation,
    /// parse errors, sidecar crashes, or model-side errors.
    pub success: bool,
    /// Plain text accumulated from `text_delta` events. Empty when
    /// `success == false`.
    pub assistant_text: String,
    /// Specific reason for failure — timeout vs. runner error vs.
    /// sidecar error event. `None` on success. Tasks surface this
    /// verbatim into `workflow_runs.error_message` so the history
    /// dialog can show what actually went wrong instead of a fixed
    /// "workflow run did not finish cleanly" placeholder.
    pub error_detail: Option<String>,
}

pub async fn run<T: BackgroundAgentTask + ?Sized>(
    task: &T,
    deps: &TaskDeps,
) -> Result<TaskOutcome> {
    let kind = task.kind();
    let thread = deps
        .chat_threads
        .create(NewChatThread {
            title: Some(format!("[{}] {}", kind.as_str(), ulid::Ulid::new())),
            bound_model_id: deps.model_id.clone(),
            bound_api_shape: deps.api_shape,
            kind: ChatThreadKind::System,
            system_task: Some(kind),
        })
        .await?;
    let thread_id = thread.id;

    // Per-run plain-text log file. Opens lazily on first write so a
    // task that fails before the sidecar spawns doesn't leave an
    // empty file behind.
    let logger = Arc::new(TaskLogger::open(&deps.app_data_dir, kind, &thread_id));
    logger.line(
        "info",
        format!(
            "task started: kind={} thread_id={} model={} (api_shape={}, thinking={})",
            kind.as_str(),
            thread_id,
            deps.model_id,
            deps.api_shape.as_str(),
            deps.thinking_level.as_str(),
        ),
    );
    tracing::info!(
        task = kind.as_str(),
        thread_id = %thread_id,
        log_path = %logger.path().display(),
        "background_agent_task.log_opened"
    );

    task.before_run(deps, &thread_id);

    let user_msg = task.initial_user_message();
    logger.block("initial user message", &user_msg);
    logger.block("system prompt", &task.system_prompt());
    logger.line(
        "info",
        format!("tool whitelist: {}", task.tool_whitelist().join(", ")),
    );

    deps.chat_messages
        .insert_user_message(NewUserMessage {
            thread_id: thread_id.clone(),
            text: user_msg.clone(),
            focus_context: None,
        })
        .await?;
    let placeholder = deps
        .chat_messages
        .start_assistant_message(&thread_id)
        .await?;

    // `std::sync::Mutex`, not `tokio::sync::Mutex`. The channel
    // callback (`build_capturing_channel`) runs synchronously from the
    // sidecar's stdout pump and needs to grab this lock without
    // awaiting. `tokio::sync::Mutex::blocking_lock()` has a runtime-
    // detection safety check that PANICS the moment it's called from
    // inside a tokio task — including a nested `tauri::async_runtime
    // ::spawn`, which we now use for the watchdog isolation below.
    // The critical section here is tiny (push to a String / set an
    // Option) and never awaits, so a plain std mutex is the right
    // choice. Real incident 2026-05-23 10:23 → 10:26: every single
    // session_memory_learning + persona_distill task panicked at
    // `blocking_lock()` until the lock type was changed.
    let buffer: Arc<Mutex<TextSink>> = Arc::new(Mutex::new(TextSink::default()));
    let channel = build_capturing_channel(buffer.clone(), logger.clone());
    let emitter = StreamEmitter::new(channel);

    let rpc_deps = RpcDeps {
        app: deps.app.clone(),
        frames_repo: deps.frames_repo.clone(),
        notes_repo: Some(deps.notes_repo.clone()),
        chat_threads: Some(deps.chat_threads.clone()),
        chat_messages: Some(deps.chat_messages.clone()),
        thread_id: thread_id.clone(),
        pending: deps.bridge_pending.clone(),
        permission_target: "main".to_string(),
        app_data_dir: deps.app_data_dir.clone(),
        // Background tasks may call schedule_task too (e.g. a future
        // "session learner notices a recurring pattern and proposes a
        // reminder" path) — wire through whatever AppState had.
        workflow_store: deps.workflow_store.clone(),
    };

    let tools_owned: Vec<String> = task.tool_whitelist();

    let cancel = Arc::new(Notify::new());

    // Hard wall-clock cap. Earlier versions tried `cancel.notify_one()`
    // from a parallel watchdog, then `tokio::time::timeout` wrapping
    // the runner future — neither survived contact with reality. The
    // failure mode was always the same: the runner future would park
    // at an `.await` (sidecar stdout read, RPC reply, …) that never
    // got woken, AND the timeout future was being polled by the same
    // task, so the timer wake-up couldn't fire either. Real incident
    // 2026-05-22 15:11:25 → 2026-05-23 18:00 (~27h): session_learner
    // sidecar (pid 47695) died seconds after `agent_start`, but the
    // runner's stdout-pump future never returned, the 5-min
    // `tokio::time::timeout` never fired, and the single-flight queue
    // was wedged with 7 items behind it indefinitely.
    //
    // The bulletproof design isolates the runner from the watchdog:
    //
    //   1. Spawn the runner on its OWN `tauri::async_runtime` task.
    //      Owns all args (no borrows from `deps` or local state).
    //   2. On the worker_loop task (our current task), `select!` the
    //      JoinHandle against a `tokio::time::sleep`. The sleep is
    //      polled here, not where the runner is wedged, so its timer
    //      wake-up always fires.
    //   3. On timeout, `handle.abort()`. tokio drops the future
    //      regardless of whether it's at an await point or has any
    //      pending wakers. The Drop chain releases the `Child` handle
    //      inside the runner; `kill_on_drop(true)` (set on the sidecar
    //      spawn in `runner/corivo.rs`) SIGKILLs the sidecar process
    //      unconditionally.
    //   4. Best-effort `handle.await` after abort so the Drop has a
    //      chance to propagate before we move on; this should return
    //      almost immediately with `Err(JoinError::Cancelled)`.
    //
    // `tauri::async_runtime::spawn` (not `tokio::spawn`) is required
    // here: nesting a bare `tokio::spawn` inside Tauri's managed
    // runtime panicked with "Cannot block the current thread from
    // within a runtime" the moment the inner future touched
    // tauri-plugin-notification. The tauri wrapper grabs the right
    // runtime handle first.
    let thread_id_for_run = thread_id.clone();
    let auth_for_run = clone_auth(&deps.auth);
    let model_id_for_run = deps.model_id.clone();
    let api_shape = deps.api_shape;
    let thinking_level = deps.thinking_level;
    let compaction_model_id = deps.compaction_model_id.clone();
    let system_prompt_for_run = task.system_prompt();
    let sessions_dir = deps.sessions_dir.clone();
    let bundled_skills_dir = deps.bundled_skills_dir.clone();
    let cloud_session = deps.cloud_session.clone();
    let mut handle: tauri::async_runtime::JoinHandle<crate::error::Result<()>> =
        tauri::async_runtime::spawn(async move {
            // `tools_native` borrows from `tools_owned`; reconstruct it
            // inside the spawn so the Vec<String> stays owned by the
            // task that uses it.
            let tools_native: Vec<&str> =
                tools_owned.iter().map(|s| s.as_str()).collect();
            run_turn_with_tools(
                &thread_id_for_run,
                user_msg,
                auth_for_run,
                model_id_for_run,
                api_shape,
                thinking_level,
                compaction_model_id,
                Some(system_prompt_for_run),
                None,
                sessions_dir,
                bundled_skills_dir,
                Vec::new(),
                Vec::<Value>::new(),
                rpc_deps,
                &emitter,
                cancel,
                &tools_native,
                cloud_session,
            )
            .await
        });

    let (run_result, timed_out): (crate::error::Result<()>, bool) = tokio::select! {
        // Prefer the runner's natural completion over the watchdog
        // when both are ready in the same poll cycle.
        biased;
        res = &mut handle => match res {
            Ok(inner) => (inner, false),
            Err(join_err) => (
                Err(crate::error::CorivoError::Internal(format!(
                    "background task join error: {join_err}"
                ))),
                false,
            ),
        },
        _ = tokio::time::sleep(BACKGROUND_TASK_TIMEOUT) => {
            tracing::warn!(
                task = kind.as_str(),
                thread_id = %thread_id,
                timeout_secs = BACKGROUND_TASK_TIMEOUT.as_secs(),
                "background_agent_task.timeout_fired_abort_runner"
            );
            handle.abort();
            // Wait briefly for the abort to propagate so the sidecar
            // Child::drop (with kill_on_drop=true) actually fires
            // SIGKILL before we tell the user "已超时". Resolves
            // ~instantly to Err(JoinError::Cancelled).
            let _ = (&mut handle).await;
            (
                Err(crate::error::CorivoError::Internal(
                    "background agent task timed out".to_string(),
                )),
                true,
            )
        }
    };

    let (assistant_text, errored) = {
        // `unwrap_or_else(|p| p.into_inner())` so a panic in the
        // callback (poisoned mutex) doesn't cascade into "task
        // mysteriously can't read its own buffer". The data we wrote
        // before the panic is still valid; we'd rather surface it.
        let snap = buffer.lock().unwrap_or_else(|p| p.into_inner());
        (snap.text.clone(), snap.errored.clone())
    };
    let success = run_result.is_ok() && errored.is_none() && !timed_out;

    // Computed below for both `finalize_assistant_message` (technical
    // sink) and `TaskOutcome.error_detail` (user-facing sink). Build it
    // once; on success it's None.
    let error_detail = if success {
        None
    } else if timed_out {
        Some(format!(
            "运行超时(>{}秒未完成)。Sidecar 可能在等上游 LLM 响应。",
            BACKGROUND_TASK_TIMEOUT.as_secs()
        ))
    } else {
        Some(match (&run_result, errored.as_deref()) {
            (Err(e), _) => e.to_string(),
            (_, Some(msg)) => msg.to_string(),
            (_, None) => "运行未正常结束(无 finish 事件)".to_string(),
        })
    };

    let outcome = TaskOutcome {
        thread_id: thread_id.clone(),
        success,
        assistant_text: assistant_text.clone(),
        error_detail: error_detail.clone(),
    };

    // Always finalize the placeholder row so the diagnostic panel
    // shows a real chat_messages entry regardless of outcome.
    let blocks = if assistant_text.is_empty() {
        Vec::new()
    } else {
        vec![ContentBlock::Text {
            text: assistant_text.clone(),
        }]
    };
    let status = if success {
        MessageStatus::Complete
    } else {
        MessageStatus::Error
    };
    let _ = deps
        .chat_messages
        .finalize_assistant_message(FinalizeAssistant {
            message_id: placeholder.id,
            content_blocks: blocks,
            cited_frame_ids: Vec::new(),
            status,
            // Re-use the same text we expose to the task via
            // `outcome.error_detail` so the chat diagnostics panel
            // and the workflows history dialog say the same thing.
            error_message: error_detail.clone(),
            finish_reason: Some(if success { "EndTurn" } else { "Error" }.to_string()),
            usage: None,
        })
        .await;

    // Final assistant text — record verbatim (preview-truncated by
    // the logger) so you can see what the model actually returned.
    if !assistant_text.is_empty() {
        logger.block("final assistant text", &assistant_text);
    }
    if !success {
        // `error_detail` is `Some(...)` whenever `!success`; pattern
        // back to a borrow for the log line + tracing span.
        let err_summary = error_detail.as_deref().unwrap_or("(no detail)");
        logger.line("error", format!("run failed: {err_summary}"));
        tracing::warn!(
            task = kind.as_str(),
            thread_id = %thread_id,
            timed_out,
            errored = ?errored,
            run_err = ?run_result.as_ref().err().map(|e| e.to_string()),
            "background_agent_task.run_failed"
        );
    }

    // Hand the partial outcome to the task even on failure — most
    // tasks want to leave the checkpoint alone, but some (e.g. error
    // bookkeeping) may still record something.
    let consume_result = task.consume_output(assistant_text, &outcome, deps).await;
    match &consume_result {
        Ok(_) => {
            logger.line("info", "consume_output: ok");
        }
        Err(error) => {
            logger.line("warn", format!("consume_output failed: {error}"));
            tracing::warn!(
                ?error,
                task = kind.as_str(),
                thread_id = %thread_id,
                "background_agent_task.consume_output_failed"
            );
        }
    }

    logger.line(
        "info",
        format!(
            "task ended: success={} elapsed_ms={}",
            success,
            logger.elapsed_ms(),
        ),
    );

    Ok(outcome)
}

#[derive(Default)]
struct TextSink {
    text: String,
    errored: Option<String>,
}

fn build_capturing_channel(
    buffer: Arc<Mutex<TextSink>>,
    logger: Arc<TaskLogger>,
) -> Channel<String> {
    Channel::new(move |body| {
        let raw_body = match body {
            tauri::ipc::InvokeResponseBody::Json(s) => s,
            tauri::ipc::InvokeResponseBody::Raw(bytes) => {
                String::from_utf8_lossy(&bytes).to_string()
            }
        };
        // Tauri's `Channel<TSend>::send(data)` calls
        // `data.body()`, which for any `T: Serialize` (including
        // `String`) runs `serde_json::to_string(&self)` — that
        // DOUBLE-encodes the line into a JSON string literal
        // (`"\"{…}\""`), so `from_str::<Value>` would yield
        // `Value::String(...)` and `value.get("type")` would always
        // be None. Decode the outer literal back to the raw line
        // before parsing as an object. Real incident 2026-05-25:
        // every daily-review run "succeeded" with `assistant_text=""`
        // because every text-delta event silently fell into the
        // catch-all match arm; the workflow history showed "本次运行
        // 未产出内容" while the model had actually emitted 140 output
        // tokens. `session_memory_learning` / `persona_distill`
        // didn't hit this because they consume their work via tool
        // side-effects (save_note), not by reading `assistant_text`.
        let line = serde_json::from_str::<String>(&raw_body).unwrap_or(raw_body);
        let trimmed = line.trim_end_matches('\n').to_string();
        // Accumulate text-delta into the in-memory sink for
        // consume_output. Everything else gets surfaced verbatim into
        // the per-run log file via render_sidecar_event.
        //
        // `std::sync::Mutex::lock()` — synchronous, no runtime check,
        // safe to call from inside this callback regardless of which
        // task is driving the sidecar's stdout pump. See the
        // declaration site above for the historical incident.
        let mut snap = buffer.lock().unwrap_or_else(|p| p.into_inner());
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&trimmed) {
            match value.get("type").and_then(|t| t.as_str()) {
                Some("text-delta") => {
                    if let Some(text) = value.get("text").and_then(|t| t.as_str()) {
                        snap.text.push_str(text);
                    }
                }
                Some("error") => {
                    snap.errored = value
                        .get("message")
                        .and_then(|m| m.as_str())
                        .map(str::to_string)
                        .or(Some("unknown sidecar error".to_string()));
                }
                _ => {}
            }
        }
        drop(snap);
        if let Some(rendered) = render_sidecar_event(&trimmed) {
            logger.line("event", rendered);
        }
        Ok(())
    })
}

/// Mirror of the parse logic inside `build_capturing_channel`'s
/// callback. Extracted so we can unit-test the decode path without
/// needing a Tauri Channel + Webview. Keep in lockstep with the
/// inline version.
#[cfg(test)]
fn decode_channel_line(raw_body: &str) -> serde_json::Value {
    // First strip the outer JSON string literal that
    // `Channel<String>::send` produces (`"{…}"` → `{…}`).
    let line = serde_json::from_str::<String>(raw_body).unwrap_or_else(|_| raw_body.to_string());
    let trimmed = line.trim_end_matches('\n').to_string();
    serde_json::from_str::<serde_json::Value>(&trimmed).unwrap_or(serde_json::Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::ipc::{InvokeResponseBody, IpcResponse};

    /// Regression: Tauri's `Channel<String>::send` runs
    /// `serde_json::to_string(&self)` on the String, producing a JSON
    /// string LITERAL — not the raw JSON object. Before the decode
    /// fix, `serde_json::from_str::<Value>(s)` returned
    /// `Value::String(_)` and `value.get("type")` was always None,
    /// causing every text-delta event to fall through the catch-all
    /// match arm. Real incident 2026-05-25.
    #[test]
    fn channel_string_double_encodes_then_decodes_to_event_object() {
        // What StreamEmitter sends:
        let wire_line = r#"{"type":"text-delta","text":"hello"}"#.to_string() + "\n";

        // What Tauri's Channel<String>::send produces internally
        // (blanket IpcResponse for T: Serialize):
        let body = wire_line.clone().body().expect("body() should succeed");
        let raw = match body {
            InvokeResponseBody::Json(s) => s,
            InvokeResponseBody::Raw(_) => panic!("expected Json body"),
        };

        // The raw body MUST be a quoted JSON string literal, not the
        // bare object — this is what tripped the original parser.
        assert!(
            raw.starts_with('"') && raw.ends_with('"'),
            "Channel<String> should produce a JSON string literal, got: {raw}",
        );

        // The decode helper round-trips correctly.
        let value = decode_channel_line(&raw);
        assert_eq!(
            value.get("type").and_then(|t| t.as_str()),
            Some("text-delta"),
        );
        assert_eq!(
            value.get("text").and_then(|t| t.as_str()),
            Some("hello"),
        );
    }

    /// Defensive: if a future code path sends already-decoded JSON
    /// (no outer quotes), `decode_channel_line` should still recover
    /// the object instead of returning Null. The fallback in the
    /// helper is `unwrap_or(raw_body)` so an unwrapped object passes
    /// through unchanged.
    #[test]
    fn decode_channel_line_handles_unwrapped_json() {
        let raw = r#"{"type":"text-delta","text":"world"}"#;
        let value = decode_channel_line(raw);
        assert_eq!(value.get("text").and_then(|t| t.as_str()), Some("world"));
    }
}

fn clone_auth(auth: &CorivoAuth) -> CorivoAuth {
    match auth {
        CorivoAuth::CorivoProxy {
            gateway_url,
            api_key,
        } => CorivoAuth::CorivoProxy {
            gateway_url: gateway_url.clone(),
            api_key: api_key.clone(),
        },
        CorivoAuth::Byok { base_url, api_key } => CorivoAuth::Byok {
            base_url: base_url.clone(),
            api_key: api_key.clone(),
        },
        CorivoAuth::Chatgpt {
            access_token,
            account_id,
        } => CorivoAuth::Chatgpt {
            access_token: access_token.clone(),
            account_id: account_id.clone(),
        },
    }
}
