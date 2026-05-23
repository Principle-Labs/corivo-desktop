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

use std::sync::Arc;

use serde_json::Value;
use tauri::ipc::Channel;
use tokio::sync::{Mutex, Notify};

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
    };

    let tools_owned: Vec<String> = task
        .tool_whitelist()
        .iter()
        .map(|s| s.to_string())
        .collect();
    let tools_native: Vec<&str> = tools_owned.iter().map(|s| s.as_str()).collect();

    let cancel = Arc::new(Notify::new());

    let run_result = run_turn_with_tools(
        &thread_id,
        user_msg,
        clone_auth(&deps.auth),
        deps.model_id.clone(),
        deps.api_shape,
        deps.thinking_level,
        deps.compaction_model_id.clone(),
        Some(task.system_prompt()),
        None,
        deps.sessions_dir.clone(),
        deps.bundled_skills_dir.clone(),
        Vec::new(),
        Vec::<Value>::new(),
        rpc_deps,
        &emitter,
        cancel,
        &tools_native,
        Some(deps.cloud_session.clone()),
    )
    .await;

    let (assistant_text, errored) = {
        let snap = buffer.lock().await;
        (snap.text.clone(), snap.errored.clone())
    };
    let success = run_result.is_ok() && errored.is_none();

    let outcome = TaskOutcome {
        thread_id: thread_id.clone(),
        success,
        assistant_text: assistant_text.clone(),
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
    let error_message = if success {
        None
    } else {
        Some(match (&run_result, errored.as_deref()) {
            (Err(e), _) => e.to_string(),
            (_, Some(msg)) => msg.to_string(),
            (_, None) => "background agent task ended without finish event".to_string(),
        })
    };
    let _ = deps
        .chat_messages
        .finalize_assistant_message(FinalizeAssistant {
            message_id: placeholder.id,
            content_blocks: blocks,
            cited_frame_ids: Vec::new(),
            status,
            error_message,
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
        let err_summary = match (&run_result, errored.as_deref()) {
            (Err(e), _) => e.to_string(),
            (_, Some(msg)) => msg.to_string(),
            (_, None) => "background agent task ended without finish event".to_string(),
        };
        logger.line("error", format!("run failed: {err_summary}"));
        tracing::warn!(
            task = kind.as_str(),
            thread_id = %thread_id,
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
        let line = match body {
            tauri::ipc::InvokeResponseBody::Json(s) => s,
            tauri::ipc::InvokeResponseBody::Raw(bytes) => {
                String::from_utf8_lossy(&bytes).to_string()
            }
        };
        let trimmed = line.trim_end_matches('\n').to_string();
        // Accumulate text-delta into the in-memory sink for
        // consume_output. Everything else gets surfaced verbatim into
        // the per-run log file via render_sidecar_event.
        let mut snap = buffer.blocking_lock();
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
    }
}
