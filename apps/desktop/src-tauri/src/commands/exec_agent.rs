//! Tauri command surface for the executive agent.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Hard ceiling for a single `PrivacyFilter::classify_and_enforce` call
/// at egress. Local ONNX inference on a healthy machine is sub-second;
/// when it blows past this it's almost certainly a system-level issue
/// (memory pressure → swap-thrash, ort op deadlock, antivirus on the
/// model file). 5s is long enough that a normal slow path still
/// finishes, short enough that a hung enforce no longer blocks the
/// whole turn for minutes. Timeout falls back to the original text +
/// WARN log so the user always gets a response.
const PRIVACY_FILTER_TIMEOUT: Duration = Duration::from_secs(5);

use serde::Deserialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tokio::sync::Notify;

use crate::commands::config::AppState;
use crate::domain::chat::ChatThreadKind;
use crate::domain::config::{ApiShape, ExecAgentAuthMode, Language};
use crate::services::exec_agent::rpc_server::RpcDeps;
use crate::services::exec_agent::runner::FocusContextInput;
use crate::services::exec_agent::runtime::{chatgpt_model_id, resolve_user_turn_runtime};
use crate::services::exec_agent::{
    load_local_context, run_turn, CorivoRunInput, LoadInputs, PermissionReply,
};
use crate::services::memory::{MemoryService, RecallRequest};
use crate::services::recall::{FinishReason, StreamEmitter};

/// Optional focus context Quick Ask attaches to the user message.
/// `/ask` doesn't pass this — the model sees the user's question alone.
#[derive(Debug, Deserialize)]
pub struct FocusContextPayload {
    /// ULID of the frame this user message is anchored to. Each Quick
    /// Ask message carries its own frame so that conversation history
    /// preserves "what was on screen when the user asked this". Emitted
    /// up-front via `cited-frames` so it lands on the assistant turn's
    /// `cited_frame_ids`; also persisted on the user message via the
    /// `user_cited_frame_ids` arg of `chat_persist_turn`.
    pub frame_id: String,
    pub summary: String,
    pub primary_text: String,
    /// User's currently-highlighted text on the focused window. When
    /// non-empty, the prompt surfaces it as a separate "用户高亮选区"
    /// block so the model can distinguish "what's on screen" from
    /// "what the user is asking about specifically".
    #[serde(default)]
    pub selection: Option<String>,
}

/// Drive one chat turn through the corivo-agent sidecar.
///
/// `assistant_message_id` is the `status='streaming'` row the frontend
/// pre-created via `chat_assistant_message_start`. We stamp
/// `model_used` on it once `resolve_user_turn_runtime` decides the alias.
/// The active model is read from `model_catalog.active_entry()` —
/// switched via `models_set_active_model`, not per-turn.
#[tauri::command]
pub async fn exec_agent_send(
    app: AppHandle,
    state: State<'_, AppState>,
    thread_id: String,
    content: String,
    focus_context: Option<FocusContextPayload>,
    stream: Channel<String>,
    assistant_message_id: String,
) -> Result<(), String> {
    let send_started_at = Instant::now();
    tracing::info!(
        target: "exec_agent",
        thread_id = %thread_id,
        assistant_message_id = %assistant_message_id,
        content_chars = content.chars().count(),
        has_focus_context = focus_context.is_some(),
        focus_frame_id = focus_context.as_ref().map(|f| f.frame_id.as_str()).unwrap_or(""),
        "exec_agent.send.entry"
    );

    // exec_agent no longer requires the long-lived UDS bridge: the
    // permission-resolution channel lives on AppState directly so the
    // agent flow keeps working on Windows (where the bridge can't bind).
    // External corivo-mcp clients still need the bridge, but that's a
    // separate code path that just won't be reachable from MCP-clients
    // on platforms where the bridge is None.
    let permission_pending = state.permission_pending.clone();

    let cfg_snapshot = state.config_service.get();

    // Snapshot the user's reply-language preference for this turn. Read
    // *once* up front so a config flip mid-turn doesn't split a single
    // response between languages.
    let response_language = cfg_snapshot.app.response_language;

    // Assemble system_prompt_extra: persistent notes block + recalled
    // memory block + Agent.md + Tools.md (memory-system-spec §6 / §8 / §9).
    let local_context_started_at = Instant::now();
    let memory_service_owned = match (
        state.frames_repo.as_ref(),
        state.notes_repo.as_ref(),
        state.chat_messages.as_ref(),
    ) {
        (Some(frames), Some(notes), Some(messages)) => Some(MemoryService::new(
            frames.clone(),
            notes.clone(),
            messages.clone(),
        )),
        _ => None,
    };
    let mut recall_request = RecallRequest::new(content.clone());
    recall_request.thread_id = Some(thread_id.clone());
    let extra_system_prompt = match app.path().app_data_dir() {
        Ok(dir) => {
            let notes_ref: Option<&dyn crate::db::repos::notes::NotesRepo> =
                state.notes_repo.as_deref();
            load_local_context(
                &dir,
                LoadInputs {
                    notes: notes_ref,
                    memory: memory_service_owned.as_ref(),
                    recall: Some(recall_request),
                },
            )
            .await
        }
        Err(e) => {
            tracing::warn!(error = %e, "exec_agent.app_data_dir_failed");
            None
        }
    };
    // Append a small response-language directive so the model honors the
    // user's pick from Settings → 常规 → 模型回复语言. Always wins over
    // any standing instruction in Soul.md/Agent.md/Tools.md because it's
    // last in the appended prompt.
    let language_directive = response_language_directive(response_language);
    let extra_system_prompt = Some(match extra_system_prompt {
        Some(prompt) => format!("{prompt}\n\n---\n\n{language_directive}"),
        None => language_directive,
    });
    tracing::info!(
        target: "exec_agent",
        thread_id = %thread_id,
        phase_ms = elapsed_ms(local_context_started_at),
        total_ms = elapsed_ms(send_started_at),
        extra_system_prompt_chars = extra_system_prompt.as_ref().map(|s| s.len()).unwrap_or(0),
        "exec_agent.send.local_context_done"
    );

    let emitter = StreamEmitter::new(stream);

    // Surface the per-message frame as a citation up-front so the
    // assistant turn's `cited_frame_ids` includes it without relying on
    // the model to call `recall_screen_history`. Quick Ask anchors every
    // message to the frame the user was looking at when they sent it
    // (CO-3); `/ask` doesn't pass `focus_context` so this is a no-op
    // there. Send-failures here are logged-and-continue: an empty
    // citation is strictly better than aborting the turn.
    if let Some(focus) = focus_context.as_ref() {
        if !focus.frame_id.is_empty() {
            if let Err(error) = emitter.cited_frames(&[focus.frame_id.clone()]) {
                tracing::warn!(?error, "exec_agent.cited_frames_emit_failed");
            }
        }
    }

    let frames_repo = state
        .frames_repo
        .as_ref()
        .cloned()
        .ok_or_else(|| "frames repo not initialized".to_string())?;
    // Quick Ask sends carry a `focus_context`; /ask doesn't. Use that as
    // the source signal so the permission prompt surfaces in the window
    // the user actually drove the turn from.
    let permission_target = if focus_context.is_some() {
        "quick-ask".to_string()
    } else {
        "main".to_string()
    };
    let deps = RpcDeps {
        app: app.clone(),
        frames_repo,
        notes_repo: state.notes_repo.as_ref().cloned(),
        chat_threads: state.chat_threads.as_ref().cloned(),
        chat_messages: state.chat_messages.as_ref().cloned(),
        thread_id: thread_id.clone(),
        // Share the AppState pending map so the existing
        // `exec_agent_permission_reply` Tauri command resolves
        // requests originating from either the rpc_server or (when
        // running) external corivo-mcp clients via the UDS bridge.
        pending: permission_pending,
        permission_target,
        app_data_dir: app.path().app_data_dir().unwrap_or_default(),
        workflow_store: state.workflow_store.as_ref().cloned(),
        privacy_filter: state.privacy_filter.clone(),
    };

    // Spec §8.2: read the thread row for `bound_model_id` /
    // `bound_api_shape` rather than reading the live Settings
    // pick. A mid-conversation Settings change must not derail
    // the active turn.
    let thread_load_started_at = Instant::now();
    let threads = state
        .chat_threads
        .as_ref()
        .cloned()
        .ok_or_else(|| "chat threads repo not initialized".to_string())?;
    let thread = threads
        .by_id(&thread_id)
        .await
        .map_err(|e| format!("Failed to load chat thread: {e}"))?
        .ok_or_else(|| format!("chat thread not found: {thread_id}"))?;

    // Guard: a kind='system' thread is owned by a background agent task
    // (session learner, persona distill, scheduled workflow). Its session
    // jsonl carries the task's private instructions and tool results;
    // letting a user chat send reuse that context produces garbled
    // assistant output (real incident 2026-05-26 13:19). The sidebar
    // already filters these out, so the only way to get here is a
    // stale-data row or a programmatic caller — refuse explicitly so
    // the failure is loud instead of silently corrupting the run.
    if !matches!(thread.kind, ChatThreadKind::User) {
        tracing::warn!(
            thread_id = %thread_id,
            kind = ?thread.kind,
            system_task = ?thread.system_task,
            "exec_agent.send.rejected_system_thread"
        );
        return Err(format!(
            "chat thread {thread_id} is owned by a background task and can't accept user sends"
        ));
    }

    // Strip the legacy `corivo:` prefix from any bound_model_id rows
    // written before we discovered the gateway (llm.eiart.top) is a
    // passthrough that only accepts upstream-native ids. Forward-compat
    // for rows already on disk; new inserts already use bare ids per
    // services/model_catalog::seeded_demo_models.
    fn strip_corivo_prefix(id: String) -> String {
        match id.strip_prefix("corivo:") {
            Some(rest) => rest.to_string(),
            None => id,
        }
    }
    // Both are `let mut` because the Chatgpt-mode self-heal below may
    // rewrite the thread's frozen pair when the user has switched modes
    // since thread-create time — we want subsequent log lines to reflect
    // the repaired binding, not the stale one we loaded from disk.
    let mut bound_upstream = strip_corivo_prefix(thread.bound_model_id);
    let mut bound_api_shape = thread.bound_api_shape;
    tracing::info!(
        target: "exec_agent",
        thread_id = %thread_id,
        phase_ms = elapsed_ms(thread_load_started_at),
        total_ms = elapsed_ms(send_started_at),
        bound_api_shape = ?bound_api_shape,
        bound_upstream = %bound_upstream,
        "exec_agent.send.thread_loaded"
    );

    // Resolve the runtime trio. In CorivoProxy mode this may fall
    // back to default_alias when the thread's frozen pair no longer
    // matches any granted alias (e.g. seeded demo, admin revoke);
    // the returned `model_id` / `api_shape` are what we send to
    // sub2api and pi-ai, NOT the (possibly stale) thread fields.
    let runtime_started_at = Instant::now();
    let runtime = resolve_user_turn_runtime(
        &cfg_snapshot,
        Some(state.cloud.session.clone()),
        state.model_catalog.as_ref().cloned(),
        Some(state.config_service.clone()),
        &bound_upstream,
        thread.bound_api_shape,
    )
    .await?;
    let thread_model_id = runtime.model_id;
    let effective_api_shape = runtime.api_shape;
    let auth = runtime.auth;
    let compaction_model_id = strip_corivo_prefix(runtime.compaction_model_id);
    tracing::info!(
        target: "exec_agent",
        thread_id = %thread_id,
        phase_ms = elapsed_ms(runtime_started_at),
        total_ms = elapsed_ms(send_started_at),
        effective_model_id = %thread_model_id,
        effective_api_shape = ?effective_api_shape,
        auth_kind = auth.kind_label(),
        alias = runtime.alias.as_deref().unwrap_or(""),
        "exec_agent.send.runtime_resolved"
    );

    // Self-heal stale Chatgpt-mode thread bindings.
    //
    // `chatgpt.com/backend-api/codex/responses` accepts exactly one
    // model id (`chatgpt_model_id()`); `resolve_chatgpt_runtime` already
    // forces that id at send time, so the request itself is fine. But
    // if the thread was created under CorivoProxy / BYOK and the user
    // later switched to Chatgpt mode, the frozen `(bound_model_id,
    // bound_api_shape)` on the row keeps diverging from what we actually
    // send — every turn surfaces a misleading
    // `bound_upstream=… / effective_model_id=gpt-5.4` log line.
    //
    // Rewrite the row once on first send under the new mode and update
    // the locals so the `exec_agent.send.resolved` line below shows the
    // repaired pair. Logged-and-swallowed: if the DB write fails the
    // turn still proceeds with the correct effective binding.
    if matches!(cfg_snapshot.exec_agent.auth_mode, ExecAgentAuthMode::Chatgpt) {
        let canonical_model = chatgpt_model_id();
        let canonical_shape = ApiShape::OpenaiResponses;
        if bound_upstream != canonical_model || bound_api_shape != canonical_shape {
            match threads
                .rebind(&thread_id, canonical_model, canonical_shape)
                .await
            {
                Ok(()) => {
                    tracing::info!(
                        target: "exec_agent",
                        thread_id = %thread_id,
                        previous_bound_upstream = %bound_upstream,
                        previous_bound_api_shape = ?bound_api_shape,
                        new_bound_upstream = %canonical_model,
                        new_bound_api_shape = ?canonical_shape,
                        "exec_agent.send.thread_rebound_for_chatgpt"
                    );
                    bound_upstream = canonical_model.to_string();
                    bound_api_shape = canonical_shape;
                }
                Err(error) => {
                    tracing::warn!(
                        target: "exec_agent",
                        ?error,
                        thread_id = %thread_id,
                        "exec_agent.send.thread_rebind_failed"
                    );
                }
            }
        }
    }

    // Stamp `chat_messages.model_used` for the assistant placeholder
    // BEFORE the sidecar fires. The audit lands even on a crash, and
    // re-running is a no-op. We only have an alias in CorivoProxy
    // mode (BYOK has no directory). Logged-and-swallowed on failure:
    // a missed UPDATE leaves model_used NULL, which the UI already
    // handles as "unknown model" — strictly better than aborting the
    // turn over an audit-only write.
    if let Some(alias) = runtime.alias.as_deref() {
        if let Some(messages) = state.chat_messages.as_ref() {
            if let Err(error) = messages
                .set_assistant_model_used(&assistant_message_id, alias)
                .await
            {
                tracing::warn!(
                    target: "exec_agent",
                    ?error,
                    assistant_message_id = %assistant_message_id,
                    alias = %alias,
                    "exec_agent.set_model_used.failed"
                );
            }
        }
    }

    tracing::info!(
        target: "exec_agent",
        thread_id = %thread_id,
        bound_upstream = %bound_upstream,
        bound_api_shape = ?bound_api_shape,
        effective_model_id = %thread_model_id,
        effective_api_shape = ?effective_api_shape,
        thinking_level = cfg_snapshot.exec_agent.thinking_level.as_str(),
        auth_kind = auth.kind_label(),
        compaction_model_id = ?compaction_model_id,
        "exec_agent.send.resolved"
    );

    // Spec §8.1: resolve the jsonl session store root once per turn.
    // SessionManager opens `${sessions_dir}/${thread_id}.jsonl`. The
    // runner mkdir's recursively, but we still validate the AppData
    // resolution here so the failure mode is a clean Tauri error rather
    // than a sidecar crash log nobody will read.
    let sessions_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("exec_agent.app_data_dir_failed: {e}"))?
        .join("corivo-agent-sessions");

    let bundled_skills_dir = resolve_bundled_skills_dir(&app);

    // Build the per-turn connector token snapshot. Skipped entirely when
    // the connector framework isn't mounted (mock builds / boot races).
    // Failures inside `enabled_snapshot` (e.g. Keychain locked, refresh
    // rejected) are absorbed by the registry — the affected connector
    // simply doesn't appear in the snapshot, so the agent won't see its
    // tools this turn and the model can't accidentally call them with
    // dead credentials.
    let connectors_started_at = Instant::now();
    let connectors_snapshot = match state.connector_registry.as_ref() {
        Some(registry) => registry.enabled_snapshot().await,
        None => Vec::new(),
    };
    tracing::info!(
        target: "exec_agent",
        thread_id = %thread_id,
        phase_ms = elapsed_ms(connectors_started_at),
        total_ms = elapsed_ms(send_started_at),
        connectors_count = connectors_snapshot.len(),
        "exec_agent.send.connectors_snapshot_done"
    );

    // Per-turn list of `mcpServer`-shape connectors. Built from the
    // current config snapshot, not from `connectors_snapshot` — MCP
    // connectors don't have OAuth tokens to ship inline; the sidecar
    // re-reads them from `token_cache_dir` on its own.
    let mcp_server_specs = crate::services::connector::mcp::enabled_mcp_specs(
        &app,
        &state.config_service,
        state.cloud.connectors.as_ref(),
    );
    tracing::info!(
        target: "exec_agent",
        thread_id = %thread_id,
        total_ms = elapsed_ms(send_started_at),
        mcp_servers_count = mcp_server_specs.len(),
        sessions_dir = %sessions_dir.display(),
        bundled_skills_dir = bundled_skills_dir
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(none)".to_string()),
        "exec_agent.send.runner_input_ready"
    );

    // Hook B —— egress classify + enforce(v1600 调整后)。
    //
    // 在把 `focus_context.primary_text` / `selection` 送进 corivo_agent
    // sidecar 之前过一遍 PII classify + redact。spans 不再从 DB 拉
    // (v1500 → v1600 撤掉了 frames.ax_text_pii_spans 列),改成每次出口
    // 现算现用:classify_and_enforce 内部走 blake3(text) → spans LRU,
    // miss 时跑模型,命中时直接复用。
    //
    // settings.enabled=false 时 classify_and_enforce 内部 fast-path 短路,
    // 不会取 cache 锁、也不会跑模型,直接返回原文 —— 但为了避免每次都
    // 走一遍函数调用,先在外面也检查一遍开关。
    let focus_redacted: Option<(String, Option<String>)> =
        if let Some(focus) = focus_context.as_ref() {
            let snap = state.privacy_filter.settings_snapshot().await;
            if !snap.enabled {
                tracing::debug!(
                    target: "privacy_filter",
                    frame_id = %focus.frame_id,
                    "enforce.skipped: settings.enabled=false"
                );
                None
            } else {
                // Wrap each enforce call in `tokio::time::timeout` —— a hung
                // ort run() (memory-swap thrash / op deadlock) used to block
                // the turn here for 8+ minutes with zero log breadcrumbs.
                // On timeout we fall back to the original text: missing PII
                // redaction is strictly less harmful than the user staring
                // at a frozen spinner for the rest of the session.
                let primary = match tokio::time::timeout(
                    PRIVACY_FILTER_TIMEOUT,
                    state.privacy_filter.classify_and_enforce(&focus.primary_text),
                )
                .await
                {
                    Ok(out) => out,
                    Err(_) => {
                        tracing::warn!(
                            target: "privacy_filter",
                            frame_id = %focus.frame_id,
                            field = "primary_text",
                            text_len = focus.primary_text.len(),
                            timeout_ms = PRIVACY_FILTER_TIMEOUT.as_millis() as u64,
                            "enforce.timeout"
                        );
                        focus.primary_text.clone()
                    }
                };
                let selection = match focus.selection.as_deref() {
                    Some(s) => Some(
                        match tokio::time::timeout(
                            PRIVACY_FILTER_TIMEOUT,
                            state.privacy_filter.classify_and_enforce(s),
                        )
                        .await
                        {
                            Ok(out) => out,
                            Err(_) => {
                                tracing::warn!(
                                    target: "privacy_filter",
                                    frame_id = %focus.frame_id,
                                    field = "selection",
                                    text_len = s.len(),
                                    timeout_ms = PRIVACY_FILTER_TIMEOUT.as_millis() as u64,
                                    "enforce.timeout"
                                );
                                s.to_string()
                            }
                        },
                    ),
                    None => None,
                };
                tracing::debug!(
                    target: "privacy_filter",
                    frame_id = %focus.frame_id,
                    primary_changed = primary != focus.primary_text,
                    "enforce.ran"
                );
                Some((primary, selection))
            }
        } else {
            None
        };

    let corivo_input = CorivoRunInput {
        auth,
        thread_model_id,
        // Use the effective shape from runtime resolution, not the
        // (possibly stale) thread row — they may diverge on fallback.
        thread_api_shape: effective_api_shape,
        thinking_level: cfg_snapshot.exec_agent.thinking_level,
        compaction_model_id,
        focus_context: focus_context.as_ref().map(|f| {
            let (primary_text, selection) = match focus_redacted.as_ref() {
                Some((pt, sel)) => (pt.clone(), sel.clone()),
                None => (f.primary_text.clone(), f.selection.clone()),
            };
            FocusContextInput {
                frame_id: f.frame_id.clone(),
                summary: f.summary.clone(),
                primary_text,
                selection,
            }
        }),
        sessions_dir,
        bundled_skills_dir,
        connectors_snapshot,
        mcp_server_specs,
        deps,
    };

    // Per-turn cancel handle. We register before spawning the sidecar
    // so a `exec_agent_cancel` racing the start of the turn still finds
    // the entry; we deregister in a `defer`-ish pattern below regardless
    // of whether the turn finishes normally, errors, or is cancelled.
    //
    // If a previous turn for the same thread is somehow still in flight
    // we overwrite — concurrent turns per thread aren't supported by
    // the FE today (`isStreaming` blocks Send), and the old handle's
    // notify is a no-op once its select branch already resolved.
    let cancel_signal = Arc::new(Notify::new());
    {
        let mut map = state.pending_turns.lock().await;
        map.insert(thread_id.clone(), cancel_signal.clone());
    }

    let run_turn_started_at = Instant::now();
    let run_result = run_turn(
        &thread_id,
        content,
        corivo_input,
        extra_system_prompt,
        &emitter,
        cancel_signal,
        runtime.cloud_session,
    )
    .await;
    tracing::info!(
        target: "exec_agent",
        thread_id = %thread_id,
        phase_ms = elapsed_ms(run_turn_started_at),
        total_ms = elapsed_ms(send_started_at),
        success = run_result.is_ok(),
        "exec_agent.send.run_turn_done"
    );

    {
        let mut map = state.pending_turns.lock().await;
        map.remove(&thread_id);
    }

    if let Err(error) = run_result {
        // 用户旅程关键失败：点了"发送"但 turn 没跑完。`tracing::error!`
        // 经 sentry 层会自动上报为 Event；前面的 `exec_agent.send.entry`
        // / `exec_agent.send.resolved` 等 info 会作为 breadcrumb 一起带上。
        tracing::error!(
            target: "exec_agent",
            thread_id = %thread_id,
            error = %error,
            "exec_agent.send.turn_failed"
        );
        let _ = emitter.error(&error.to_string());
        let _ = emitter.finish(FinishReason::Error);
        return Err(error.to_string());
    }
    Ok(())
}

fn elapsed_ms(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

/// Cancel an in-flight `exec_agent_send` for `thread_id`. Returns
/// `Ok(())` whether or not a turn was actually running — the FE's
/// "Stop" button is fire-and-forget and shouldn't surface a noisy error
/// if the user clicks it after the stream already finished. The runner
/// reacts by killing the corivo-agent child and emitting a final
/// `finish` event with `reason: "cancelled"` over the stream channel.
#[tauri::command]
pub async fn exec_agent_cancel(
    state: State<'_, AppState>,
    thread_id: String,
) -> Result<(), String> {
    let map = state.pending_turns.lock().await;
    if let Some(notify) = map.get(&thread_id) {
        tracing::info!(
            target: "exec_agent",
            thread_id = %thread_id,
            "exec_agent.cancel.signalled"
        );
        notify.notify_one();
    } else {
        tracing::debug!(
            target: "exec_agent",
            thread_id = %thread_id,
            "exec_agent.cancel.no_pending"
        );
    }
    Ok(())
}

#[tauri::command]
pub async fn exec_agent_permission_reply(
    state: State<'_, AppState>,
    request_id: String,
    allow: bool,
    message: Option<String>,
) -> Result<(), String> {
    let resolved = crate::services::exec_agent::mcp_bridge::resolve_permission(
        &state.permission_pending,
        &request_id,
        PermissionReply { allow, message },
    )
    .await;
    if !resolved {
        return Err(format!(
            "no pending permission request with id {request_id}"
        ));
    }
    Ok(())
}

/// Locate the directory holding bundled skills (e.g.
/// `corivo-feedback`) shipped with the desktop app.
///
/// In a packaged build Tauri's `bundle.resources` copies
/// `apps/desktop/bundled-skills/` to
/// `Corivo.app/Contents/Resources/bundled-skills/`, which
/// `app.path().resource_dir()` resolves to.
///
/// In `app:dev` Tauri does NOT stage `bundle.resources` — the binary
/// runs straight out of `target/debug/`. We fall back to the source
/// path resolved at compile time via `CARGO_MANIFEST_DIR`
/// (= `apps/desktop/src-tauri/`).
///
/// Returns `None` when neither path is on disk; the sidecar then
/// just doesn't see any bundled skills (user-only skill sources still
/// load).
fn resolve_bundled_skills_dir(app: &AppHandle) -> Option<PathBuf> {
    if let Ok(resource_dir) = app.path().resource_dir() {
        let candidate = resource_dir.join("bundled-skills");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("bundled-skills");
    if dev.is_dir() {
        return Some(dev);
    }
    None
}

/// Render a brief response-language directive that the orchestrator
/// stitches onto the end of `system_prompt_extra`. Phrased as a hard
/// rule so it overrides the standing instructions in
/// `Agent.md` / `Soul.md` / `Tools.md`.
fn response_language_directive(language: Language) -> String {
    match language {
        Language::Zh => "# Response language\n\
请始终用简体中文回复用户。代码、命令行、文件路径、API 名称等保持原文，不要翻译。"
            .to_string(),
        Language::En => "# Response language\n\
Always reply to the user in English. Keep code, command-line snippets, file paths, and API names in their original form."
            .to_string(),
    }
}

#[cfg(test)]
mod response_language_tests {
    use super::*;

    #[test]
    fn zh_directive_mentions_chinese_reply() {
        let directive = response_language_directive(Language::Zh);
        assert!(directive.contains("中文"));
    }

    #[test]
    fn en_directive_mentions_english_reply() {
        let directive = response_language_directive(Language::En);
        assert!(directive.contains("English"));
    }

    #[test]
    fn directive_is_phrased_as_a_hard_rule() {
        for lang in [Language::Zh, Language::En] {
            let directive = response_language_directive(lang);
            // Both variants lead with the response-language header so a
            // model scanning prompt sections can pick it out.
            assert!(directive.starts_with("# Response language"));
        }
    }
}
