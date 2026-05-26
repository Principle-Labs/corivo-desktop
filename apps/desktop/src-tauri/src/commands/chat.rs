//! Chat Tauri command surface.
//!
//! Thread + message CRUD only. The actual chat turn (LLM streaming,
//! tool calls, side-effects) goes through `commands::exec_agent` —
//! everything funnels through the `corivo-agent` sidecar.
//!
//! v1200 redesign — three-step persistence (replaces `chat_persist_turn`):
//!
//! 1. `chat_user_message_create` — fires the moment the user hits send.
//!    The user's prompt persists immediately; failures downstream
//!    (network / agent crash / cancel) cannot lose it. Quick Ask
//!    attaches a focus context block; `/ask` doesn't.
//!
//! 2. `chat_assistant_message_start` — fires right before the agent
//!    spawns. Drops a `status='streaming'` placeholder row that owns
//!    a stable id + created_at; the live UI reads from a separate
//!    `liveMessages` channel for streaming visualization, so this
//!    placeholder isn't shown to the user (the `by_thread` reader
//!    filters streaming rows).
//!
//! 3. `chat_assistant_message_finalize` — fires when the stream
//!    terminates (success / error / cancel). Flips status, commits
//!    the accumulated `content_blocks` JSON, persists `usage` /
//!    `finish_reason`, bumps `updated_at`.
//!
//! On the first user-send into an unnamed (or placeholder-titled)
//! thread we also auto-name the thread from the user's question
//! (CJK-safe, ≤24 chars). The auto-name now lands in step 1, not
//! step 3 — the sidebar picks up the real title before the assistant
//! even starts streaming.

use tauri::{AppHandle, Emitter, Manager, Runtime, State};

use crate::{
    commands::config::AppState,
    db::repos::chat::{FinalizeAssistant, NewChatThread, NewUserMessage, UserFocusContext},
    domain::{
        chat::{ChatMessage, ChatThread, ChatThreadsChanged, ContentBlock, MessageStatus, Usage},
        config::{ApiShape, ExecAgentAuthMode},
    },
};

/// Best-effort cross-window broadcast of a thread mutation. Logged-and-
/// swallowed on failure: a missed event leaves the other window with a
/// stale list, which is the same failure mode this whole event channel
/// exists to fix — no point bringing down the command for it.
fn emit_threads_changed<R: Runtime>(app: &AppHandle<R>, payload: ChatThreadsChanged) {
    if let Err(error) = app.emit(ChatThreadsChanged::EVENT, &payload) {
        tracing::warn!(?error, "chat.emit_threads_changed_failed");
    }
}

#[tauri::command]
pub async fn chat_threads_list(state: State<'_, AppState>) -> Result<Vec<ChatThread>, String> {
    let threads = state
        .chat_threads
        .as_ref()
        .ok_or_else(|| "chat threads repo not initialized".to_string())?
        .clone();
    threads.list(50).await.map_err(String::from)
}

#[tauri::command]
pub async fn chat_thread_create(
    app: AppHandle,
    state: State<'_, AppState>,
    title: Option<String>,
) -> Result<ChatThread, String> {
    let threads = state
        .chat_threads
        .as_ref()
        .ok_or_else(|| "chat threads repo not initialized".to_string())?
        .clone();

    // Spec §8.2: bind the new thread to the current Settings model
    // pick. CorivoProxy mode reads `selected_model_id` + the §7.5
    // cache for `api_shape`; BYOK mode reads `byok_model` +
    // `byok_api_shape`. If neither path yields a model, we fail early
    // — better a clear "no model configured" error than spawning a
    // sidecar that immediately dies on a malformed input.
    let cfg = state.config_service.get();
    let resolved = resolve_thread_binding(&cfg, state.model_catalog.as_ref())?;

    let thread = threads
        .create(NewChatThread {
            title,
            bound_model_id: resolved.model_id,
            bound_api_shape: resolved.api_shape,
            ..NewChatThread::default()
        })
        .await
        .map_err(String::from)?;
    emit_threads_changed(
        &app,
        ChatThreadsChanged::Created {
            thread_id: thread.id.clone(),
        },
    );
    Ok(thread)
}

struct ThreadBinding {
    model_id: String,
    api_shape: ApiShape,
}

fn resolve_thread_binding(
    cfg: &crate::domain::config::Config,
    catalog: Option<&std::sync::Arc<crate::services::model_catalog::ModelCatalog>>,
) -> Result<ThreadBinding, String> {
    // Chatgpt mode: the chatgpt.com/backend-api/codex endpoint only
    // accepts a single id (see `runtime::CHATGPT_MODEL_ID`). Freeze
    // that pair onto the thread row at create time so the bound model
    // matches what `resolve_chatgpt_runtime` will actually send —
    // otherwise the log surface shows a spurious
    // `bound_upstream=<settings-pick> / effective=gpt-5.4` mismatch
    // every turn even though the network call is correct.
    if matches!(cfg.exec_agent.auth_mode, ExecAgentAuthMode::Chatgpt) {
        return Ok(ThreadBinding {
            model_id: crate::services::exec_agent::runtime::chatgpt_model_id().to_string(),
            api_shape: ApiShape::OpenaiResponses,
        });
    }

    // BYOK first — when the user has explicitly toggled to "自带 API
    // Key", the picker UI may legitimately have an empty
    // `selected_model_id`. We only fall through to the CorivoProxy
    // path when auth_mode is corivo_proxy.
    if matches!(cfg.exec_agent.auth_mode, ExecAgentAuthMode::Byok) {
        let model_id = cfg
            .exec_agent
            .byok_model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "BYOK 模式但未配置 model id —— 请到设置页填写".to_string())?
            .to_string();
        let api_shape = cfg
            .exec_agent
            .byok_api_shape
            .ok_or_else(|| "BYOK 模式但未选择 api shape —— 请到设置页填写".to_string())?;
        return Ok(ThreadBinding {
            model_id,
            api_shape,
        });
    }

    // CorivoProxy path. `selected_model_id` historically held a raw
    // upstream id; with the new directory it holds the alias the user
    // picked (e.g. `"corivo:fast"`). We resolve the alias to the
    // upstream id + client protocol once at thread-create time and
    // freeze both onto the thread row — mid-conversation directory
    // edits (admin re-points an alias) MUST NOT change in-flight
    // turns. A revoked alias surfaces here as "unknown alias"; an
    // empty cache falls back to the directory's default_alias.
    let selected = cfg
        .exec_agent
        .selected_model_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let alias = match selected {
        Some(a) => a.to_string(),
        None => catalog
            .map(|c| c.load_cached().default_alias)
            .filter(|a| !a.is_empty())
            .ok_or_else(|| "未配置任何模型 —— 请到设置页选择主模型或填写 BYOK 信息".to_string())?,
    };

    let entry = catalog
        .and_then(|c| c.find_alias(&alias))
        .or_else(|| {
            // Cache miss: fall back to default_alias before giving up.
            // Happens on first launch before login lands or when admin
            // revokes the user's previously-selected alias.
            catalog.and_then(|c| {
                let dir = c.load_cached();
                dir.managed
                    .into_iter()
                    .find(|m| m.alias == dir.default_alias)
            })
        })
        .ok_or_else(|| format!("未找到 alias「{alias}」的模型条目 —— 请刷新模型列表后重试"))?;

    Ok(ThreadBinding {
        model_id: entry.upstream_model,
        api_shape: entry.client_protocol,
    })
}

#[tauri::command]
pub async fn chat_thread_delete(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let threads = state
        .chat_threads
        .as_ref()
        .ok_or_else(|| "chat threads repo not initialized".to_string())?
        .clone();
    threads.delete(&id).await.map_err(String::from)?;

    // Spec §8.1: a thread's jsonl agent-session lives in
    // `$APPDATA/corivo-agent-sessions/{thread_id}.jsonl`. Drop it
    // alongside the DB row so a deleted thread leaves no resumable
    // agent state behind. Best-effort: a missing file (the thread
    // never produced an assistant turn so no jsonl was flushed) is
    // not an error.
    if let Ok(dir) = app.path().app_data_dir() {
        let jsonl = dir
            .join("corivo-agent-sessions")
            .join(format!("{id}.jsonl"));
        match std::fs::remove_file(&jsonl) {
            Ok(()) => {
                tracing::info!(path = %jsonl.display(), "chat.thread_delete.jsonl_removed");
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Thread never reached an assistant turn — nothing to
                // clean. Drop the trace to debug to keep info level
                // signal-to-noise high.
                tracing::debug!(path = %jsonl.display(), "chat.thread_delete.jsonl_absent");
            }
            Err(e) => {
                tracing::warn!(?e, path = %jsonl.display(), "chat.thread_delete.jsonl_remove_failed");
            }
        }
    } else {
        tracing::warn!("chat.thread_delete.app_data_dir_unresolved");
    }

    emit_threads_changed(&app, ChatThreadsChanged::Deleted { thread_id: id });
    Ok(())
}

/// v1300 · sidebar lifecycle. Toggle the thread's pinned state.
/// `pinned: true` stamps `pinned_at = now`; `false` clears it.
/// Doesn't bump `updated_at` — pin/unpin is metadata, not activity,
/// and we don't want it to leak into the recent-list ordering.
#[tauri::command]
pub async fn chat_thread_set_pinned(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    pinned: bool,
) -> Result<(), String> {
    let threads = state
        .chat_threads
        .as_ref()
        .ok_or_else(|| "chat threads repo not initialized".to_string())?
        .clone();
    threads
        .set_pinned(&id, pinned)
        .await
        .map_err(String::from)?;
    emit_threads_changed(&app, ChatThreadsChanged::Updated { thread_id: id });
    Ok(())
}

/// v1300 · sidebar lifecycle. Toggle the thread's archived state.
/// `archived: true` stamps `archived_at = now`; `false` clears it.
/// Same `updated_at` discipline as `chat_thread_set_pinned`.
#[tauri::command]
pub async fn chat_thread_set_archived(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    archived: bool,
) -> Result<(), String> {
    let threads = state
        .chat_threads
        .as_ref()
        .ok_or_else(|| "chat threads repo not initialized".to_string())?
        .clone();
    threads
        .set_archived(&id, archived)
        .await
        .map_err(String::from)?;
    emit_threads_changed(&app, ChatThreadsChanged::Updated { thread_id: id });
    Ok(())
}

#[tauri::command]
pub async fn chat_messages_by_thread(
    state: State<'_, AppState>,
    thread_id: String,
) -> Result<Vec<ChatMessage>, String> {
    let messages = state
        .chat_messages
        .as_ref()
        .ok_or_else(|| "chat messages repo not initialized".to_string())?
        .clone();
    messages.by_thread(&thread_id).await.map_err(String::from)
}

/// Placeholder title the frontend stamps on a fresh thread it creates
/// at the moment of the first user send (so the sidebar shows the
/// thread immediately while the turn is still streaming). After the
/// user's first message lands we overwrite this with a real derived
/// title — see [`needs_auto_title`]. Must stay in sync with the
/// constant in `apps/desktop/src/pages/ask/ask-page.tsx`.
pub const PLACEHOLDER_THREAD_TITLE: &str = "新会话";

/// Step 1 of the v1200 turn lifecycle: persist the user's prompt
/// immediately. Returns the canonical row (id + created_at) so the
/// frontend can correlate the streaming UI with the persisted message.
///
/// Quick Ask passes a `focus_context` to attach the on-screen frame
/// the question was anchored to (CO-3); `/ask` passes `None`. The
/// repo prepends a `FocusContext` block when present and mirrors the
/// frame id into `cited_frame_ids` so per-frame queries hit the
/// indexed column directly.
///
/// Auto-titles the thread on its first user send if the title is
/// empty / placeholder. Earlier than the legacy single-turn persist
/// (which named on assistant-finalize), so the sidebar shows a real
/// title before the assistant even starts streaming.
#[tauri::command]
pub async fn chat_user_message_create(
    app: AppHandle,
    state: State<'_, AppState>,
    thread_id: String,
    text: String,
    focus_context: Option<UserFocusContextPayload>,
) -> Result<ChatMessage, String> {
    let messages = state
        .chat_messages
        .as_ref()
        .ok_or_else(|| "chat messages repo not initialized".to_string())?
        .clone();
    let threads = state
        .chat_threads
        .as_ref()
        .ok_or_else(|| "chat threads repo not initialized".to_string())?
        .clone();

    let inserted = messages
        .insert_user_message(NewUserMessage {
            thread_id: thread_id.clone(),
            text: text.clone(),
            focus_context: focus_context.map(UserFocusContextPayload::into_repo),
        })
        .await
        .map_err(String::from)?;

    if let Some(thread) = threads.by_id(&thread_id).await.map_err(String::from)? {
        if needs_auto_title(thread.title.as_deref()) {
            let derived = derive_thread_title(&text);
            if !derived.is_empty() {
                threads
                    .rename(&thread_id, Some(derived))
                    .await
                    .map_err(String::from)?;
            }
        }
    }

    emit_threads_changed(&app, ChatThreadsChanged::Updated { thread_id });

    Ok(inserted)
}

/// Step 2 of the v1200 turn lifecycle: drop a `status='streaming'`
/// assistant placeholder row so we have a stable id to hang the
/// stream off. The placeholder is filtered out of `chat_messages_by_thread`
/// reads — only the boot-time orphan sweep ever sees it.
///
/// Doesn't emit `chat:threads-changed` — the sidebar's "上次活动"
/// already moved when the user message was inserted, no need to
/// double-fire.
#[tauri::command]
pub async fn chat_assistant_message_start(
    state: State<'_, AppState>,
    thread_id: String,
) -> Result<ChatMessage, String> {
    let messages = state
        .chat_messages
        .as_ref()
        .ok_or_else(|| "chat messages repo not initialized".to_string())?
        .clone();

    messages
        .start_assistant_message(&thread_id)
        .await
        .map_err(String::from)
}

/// Step 3 of the v1200 turn lifecycle: terminal write for the
/// assistant turn. Status moves out of `streaming` into one of
/// `complete` / `error` / `cancelled`; `content_blocks` /
/// `cited_frame_ids` / `finish_reason` / `usage` / `error_message`
/// land in their columns.
///
/// Called on every stream termination — success, error, AND user
/// cancellation. The frontend's `BlockAccumulator` builds the final
/// `content_blocks` array from the streaming events; this command
/// just commits it.
#[tauri::command]
pub async fn chat_assistant_message_finalize(
    app: AppHandle,
    state: State<'_, AppState>,
    message_id: String,
    content_blocks: Vec<ContentBlock>,
    cited_frame_ids: Option<Vec<String>>,
    status: MessageStatus,
    error_message: Option<String>,
    finish_reason: Option<String>,
    usage: Option<Usage>,
) -> Result<ChatMessage, String> {
    let messages = state
        .chat_messages
        .as_ref()
        .ok_or_else(|| "chat messages repo not initialized".to_string())?
        .clone();

    let finalized = messages
        .finalize_assistant_message(FinalizeAssistant {
            message_id,
            content_blocks,
            cited_frame_ids: cited_frame_ids.unwrap_or_default(),
            status,
            error_message,
            finish_reason,
            usage,
        })
        .await
        .map_err(String::from)?;

    emit_threads_changed(
        &app,
        ChatThreadsChanged::Updated {
            thread_id: finalized.thread_id.clone(),
        },
    );

    Ok(finalized)
}

/// Frontend-facing serialization of [`UserFocusContext`]. Mirrors the
/// `ExecAgentFocusContext` field naming the existing TS client uses
/// (snake_case via Tauri's default serde rules).
#[derive(Debug, serde::Deserialize)]
pub struct UserFocusContextPayload {
    pub frame_id: String,
    pub summary: String,
    #[serde(default)]
    pub primary_text: Option<String>,
    #[serde(default)]
    pub selection: Option<String>,
}

impl UserFocusContextPayload {
    fn into_repo(self) -> UserFocusContext {
        UserFocusContext {
            frame_id: self.frame_id,
            summary: self.summary,
            primary_text: self.primary_text,
            selection: self.selection,
        }
    }
}

/// A thread should be auto-titled on its first turn iff it is missing a
/// title, has only whitespace, or is still wearing the type-to-create
/// placeholder stamped by the frontend.
fn needs_auto_title(current: Option<&str>) -> bool {
    match current {
        None => true,
        Some(t) => {
            let trimmed = t.trim();
            trimmed.is_empty() || trimmed == PLACEHOLDER_THREAD_TITLE
        }
    }
}

/// First-line, char-truncated title for a thread. CJK-safe — counts by
/// Unicode `char` rather than UTF-8 bytes. Adds an ellipsis when the
/// source got cut off.
fn derive_thread_title(content: &str) -> String {
    const MAX_CHARS: usize = 24;
    let line = content
        .trim()
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let total = line.chars().count();
    if total == 0 {
        return String::new();
    }
    if total <= MAX_CHARS {
        return line.to_string();
    }
    let mut out: String = line.chars().take(MAX_CHARS).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::{derive_thread_title, needs_auto_title, PLACEHOLDER_THREAD_TITLE};

    #[test]
    fn needs_auto_title_handles_each_state() {
        assert!(needs_auto_title(None));
        assert!(needs_auto_title(Some("")));
        assert!(needs_auto_title(Some("   ")));
        assert!(needs_auto_title(Some(PLACEHOLDER_THREAD_TITLE)));
        assert!(needs_auto_title(Some(&format!(
            "  {PLACEHOLDER_THREAD_TITLE}  "
        ))));
        // Real titles — including manual renames — must stick.
        assert!(!needs_auto_title(Some("hello")));
        assert!(!needs_auto_title(Some("new conversation"))); // Not the CN placeholder
    }

    #[test]
    fn derive_title_short_passthrough() {
        assert_eq!(derive_thread_title("hello"), "hello");
    }

    #[test]
    fn derive_title_trims_whitespace() {
        assert_eq!(derive_thread_title("  hi  "), "hi");
    }

    #[test]
    fn derive_title_uses_first_non_empty_line() {
        assert_eq!(
            derive_thread_title("\n\n  first line\nsecond"),
            "first line"
        );
    }

    #[test]
    fn derive_title_truncates_with_ellipsis() {
        let long = "a".repeat(40);
        let title = derive_thread_title(&long);
        assert_eq!(title.chars().count(), 25); // 24 chars + ellipsis
        assert!(title.ends_with('…'));
    }

    #[test]
    fn derive_title_handles_cjk_correctly() {
        // 30 Chinese characters → truncate to 24 + ellipsis
        let long: String =
            "今天上午我都在看代码以及和团队讨论这个新功能的实现方案细节问题".to_string();
        let title = derive_thread_title(&long);
        assert_eq!(title.chars().count(), 25);
        assert!(title.ends_with('…'));
    }

    #[test]
    fn derive_title_empty_input() {
        assert_eq!(derive_thread_title(""), "");
        assert_eq!(derive_thread_title("   \n  "), "");
    }
}
