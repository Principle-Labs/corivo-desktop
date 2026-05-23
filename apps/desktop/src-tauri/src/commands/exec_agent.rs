//! Tauri command surface for the executive agent.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use serde::Deserialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tokio::sync::Notify;

use crate::commands::config::AppState;
use crate::domain::config::{ApiShape, ExecAgentAuthMode, Language};
use crate::services::exec_agent::rpc_server::RpcDeps;
use crate::services::exec_agent::runner::{CorivoAuth, FocusContextInput};
use crate::services::exec_agent::{
    load_local_context, run_turn, CorivoRunInput, LoadInputs, PermissionReply,
};
use crate::services::memory::{MemoryService, RecallRequest};
use crate::services::recall::{FinishReason, StreamEmitter};

const DEFAULT_COMPACTION_ANTHROPIC: &str = "claude-haiku-4-5";
const DEFAULT_COMPACTION_OPENAI: &str = "gpt-4o-mini";

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

/// Per-turn runtime binding: the auth bundle + the alias-derived
/// (upstream_model, api_shape) for this turn. Single-key
/// architecture: every account has ONE sub2api key whose group is
/// the model selector. The key is read from `CloudSessionService`
/// (same cloud credential pair that drives auth), the active alias is
/// read from the model directory cache.
struct RuntimeBinding {
    auth: CorivoAuth,
    /// What gets sent to sub2api as the `model` field.
    model_id: String,
    /// Drives the sidecar's adapter pick + the compaction partner
    /// fallback. Comes from the active alias's `client_protocol`.
    api_shape: ApiShape,
    /// Directory alias that actually drove this turn — stamped onto
    /// the assistant message for audit. `None` in Byok mode.
    alias: Option<String>,
}

/// Resolve the auth bundle + (model_id, api_shape, alias) for this
/// turn. CorivoProxy mode reads:
///   * host + api_key from `CloudSessionService::fetch_agent_creds()` (the
///     single per-user sub2api key minted at signup),
///   * active alias from `model_catalog.active_entry()` (set by the
///     picker via `models_set_active_model`).
/// Byok mode forwards the thread's frozen pair unchanged.
async fn resolve_corivo_runtime(
    state: &AppState,
    cfg: &crate::domain::config::Config,
    thread_upstream_model: &str,
    thread_api_shape: ApiShape,
) -> Result<RuntimeBinding, String> {
    match cfg.exec_agent.auth_mode {
        ExecAgentAuthMode::CorivoProxy => {
            let session = state.cloud.session.clone();
            let catalog = state
                .model_catalog
                .as_ref()
                .ok_or_else(|| "model directory not initialized — restart after login".to_string())?
                .clone();
            let entry = catalog.active_entry().ok_or_else(|| {
                "model directory is empty — refresh after the admin enables at least one model"
                    .to_string()
            })?;
            // Fetch creds (the long-lived per-user key); on 401 the
            // session is wiped and the UI is notified.
            let creds = session
                .fetch_agent_creds()
                .await
                .map_err(|e| e.to_string())?;
            // Align the sub2api key's group_id with the alias we're
            // about to invoke. sub2api routes by group, not by the
            // chat request's `model` field — so the picker's "current
            // alias" and the gateway's "current group" can drift
            // (notably on first turn after fresh login, before the
            // user has ever opened the picker). Cached per-process so
            // steady-state turns skip the round-trip.
            session
                .ensure_model_alias_synced(catalog.clone(), entry.alias.clone())
                .await
                .map_err(|e| {
                    format!("failed to align gateway with alias {}: {}", entry.alias, e)
                })?;
            tracing::info!(
                target: "exec_agent",
                api_host = %creds.api_host,
                alias = %entry.alias,
                upstream_model = %entry.upstream_model,
                api_shape = ?entry.client_protocol,
                api_key_len = creds.api_key.len(),
                label = ?creds.label,
                email = ?creds.email,
                "exec_agent.corivo_proxy.creds_resolved"
            );
            Ok(RuntimeBinding {
                model_id: entry.upstream_model,
                api_shape: entry.client_protocol,
                alias: Some(entry.alias),
                auth: CorivoAuth::CorivoProxy {
                    gateway_url: creds.api_host,
                    api_key: creds.api_key,
                },
            })
        }
        ExecAgentAuthMode::Byok => {
            let api_key = cfg
                .exec_agent
                .byok_key
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .ok_or_else(|| "BYOK mode is selected but no API key is configured".to_string())?;
            Ok(RuntimeBinding {
                auth: CorivoAuth::Byok {
                    base_url: cfg
                        .exec_agent
                        .byok_base_url
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                    api_key,
                },
                model_id: thread_upstream_model.to_string(),
                api_shape: thread_api_shape,
                alias: None,
            })
        }
    }
}

/// Resolve the compaction model id for a turn. The new directory
/// shape (`/v1/me/models`) does not surface a per-alias compaction
/// partner — sub2api routes the cheap-model traffic through the
/// same key, and a Corivo-wide default per protocol is good enough
/// while the catalog is small. If we later want per-alias overrides,
/// extend `ModelMeta` with `compaction_upstream_model: Option<String>`
/// in the backend and look it up here.
fn resolve_compaction_partner(api_shape: ApiShape) -> String {
    match api_shape {
        ApiShape::Anthropic => DEFAULT_COMPACTION_ANTHROPIC.to_string(),
        // Responses / Completions 共用同一个 OpenAI cheap compaction model
        // ——gpt-4o-mini 在两个 API 形态都跑得通,sub2api 按 alias 路由,
        // 这里不需要为 Responses 单列一个 id。
        ApiShape::Openai | ApiShape::OpenaiResponses => DEFAULT_COMPACTION_OPENAI.to_string(),
    }
}

/// Drive one chat turn through the corivo-agent sidecar.
///
/// `assistant_message_id` is the `status='streaming'` row the frontend
/// pre-created via `chat_assistant_message_start`. We stamp
/// `model_used` on it once `resolve_corivo_runtime` decides the alias.
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
    let bound_upstream = strip_corivo_prefix(thread.bound_model_id);
    tracing::info!(
        target: "exec_agent",
        thread_id = %thread_id,
        phase_ms = elapsed_ms(thread_load_started_at),
        total_ms = elapsed_ms(send_started_at),
        bound_api_shape = ?thread.bound_api_shape,
        bound_upstream = %bound_upstream,
        "exec_agent.send.thread_loaded"
    );

    // Resolve the runtime trio. In CorivoProxy mode this may fall
    // back to default_alias when the thread's frozen pair no longer
    // matches any granted alias (e.g. seeded demo, admin revoke);
    // the returned `model_id` / `api_shape` are what we send to
    // sub2api and pi-ai, NOT the (possibly stale) thread fields.
    let runtime_started_at = Instant::now();
    let runtime = resolve_corivo_runtime(
        &state,
        &cfg_snapshot,
        &bound_upstream,
        thread.bound_api_shape,
    )
    .await?;
    let thread_model_id = runtime.model_id;
    let effective_api_shape = runtime.api_shape;
    let auth = runtime.auth;
    let compaction_model_id = strip_corivo_prefix(resolve_compaction_partner(effective_api_shape));
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
        bound_api_shape = ?thread.bound_api_shape,
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

    // Hook B —— egress enforce(docs/privacy-filter-spec.md §7.2)。
    //
    // 在把 `focus_context.primary_text` / `selection` 送进 corivo_agent
    // sidecar 之前过一遍 PII redact。spans 从 frames.ax_text_pii_spans
    // 拉取 —— 当前 capture 还没接 classify hook(下一刀的事),所以多数
    // frame 的 spans 列是 NULL,enforce 会拿到空 spans 数组 + 用户偏好,
    // 走纯字符串路径直通返回。等 capture-time classify 上线后,这条
    // hook 不用再改就自动开始工作。
    //
    // settings.enabled=false 时 enforce 内部 fast-path 短路,不会取 cache
    // 锁、也不会去查 frames_repo —— 但为了避免每次都查 DB,先在外面也
    // 检查一遍开关,空设置直接跳过整段 lookup。
    let focus_redacted: Option<(String, Option<String>)> =
        if let Some(focus) = focus_context.as_ref() {
            let snap = state.privacy_filter.settings_snapshot().await;
            if !snap.enabled {
                None
            } else {
                let spans = fetch_pii_spans_for_frame(&state, &focus.frame_id).await;
                let primary = state
                    .privacy_filter
                    .enforce(&focus.primary_text, &spans)
                    .await;
                let selection = match focus.selection.as_deref() {
                    Some(s) => Some(state.privacy_filter.enforce(s, &spans).await),
                    None => None,
                };
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

    // Hand the runner a cloud session handle only in CorivoProxy
    // mode — that's the only mode where a sidecar `auth_failed`
    // upstream error should reach into corivo's session manager to
    // refresh creds. BYOK is the user's own key; if their upstream
    // 401s, refreshing the cloud session isn't going to fix it.
    let session_for_runner = match cfg_snapshot.exec_agent.auth_mode {
        crate::domain::config::ExecAgentAuthMode::CorivoProxy => Some(state.cloud.session.clone()),
        crate::domain::config::ExecAgentAuthMode::Byok => None,
    };

    let run_turn_started_at = Instant::now();
    let run_result = run_turn(
        &thread_id,
        content,
        corivo_input,
        extra_system_prompt,
        &emitter,
        cancel_signal,
        session_for_runner,
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

/// Pull PII spans for a frame from `frames.ax_text_pii_spans`(JSON
/// 列)。所有失败路径都退化成空 vec —— privacy filter 的 enforce 拿到
/// 空 spans 就走直通,呼叫方的 UX 不受影响。
///
/// 调用时机:`exec_agent_send` Hook B(spec §7.2),在把 focus_context
/// 的 primary_text 喂给 sidecar 之前。
async fn fetch_pii_spans_for_frame(
    state: &State<'_, AppState>,
    frame_id: &str,
) -> Vec<crate::domain::privacy::PiiSpan> {
    let Some(repo) = state.frames_repo.as_ref() else {
        return Vec::new();
    };
    let frame = match repo.by_id(frame_id).await {
        Ok(Some(f)) => f,
        Ok(None) => return Vec::new(),
        Err(error) => {
            tracing::warn!(?error, frame_id, "privacy_filter.spans_lookup_failed");
            return Vec::new();
        }
    };
    let Some(json) = frame.ax_text_pii_spans.as_deref() else {
        return Vec::new();
    };
    match serde_json::from_str::<Vec<crate::domain::privacy::PiiSpan>>(json) {
        Ok(spans) => spans,
        Err(error) => {
            tracing::warn!(?error, frame_id, "privacy_filter.spans_parse_failed");
            Vec::new()
        }
    }
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
