import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  BillingMe,
  Capabilities,
  ComposioConnectionLink,
  ComposioConnectionsResponse,
  ConnectorSummary,
  ContentBlock,
  DeleteRangeReport,
  EmailLoginOutcome,
  MessageStatus,
  ModelDirectory,
  ModelDownloadProgress,
  Note,
  NoteScope,
  NoteSourceType,
  NoteStatus,
  PrivacyModelStatus,
  PrivacySettings,
  RequestEmailCodeOutcome,
  Trigger,
  TriggerPreview,
  Usage,
  WebsiteExclusionEntry,
  WorkflowRun,
  WorkflowSaveSpec,
  WorkflowView,
} from "@corivo/shared-types";
import type {
  AuthStatus,
  AvailableSkill,
  CaptureStatus,
  ChatMessage,
  ChatThread,
  Config,
  DataPaths,
  FocusContext,
  Frame,
  HotkeyStatus,
  ListFramesArgs,
  OnboardingState,
  StorageStats,
  SystemInfo,
  TauriError,
} from "@/lib/types";

export type {
  ModelCapabilities,
  ModelDirectory,
  ModelMeta,
} from "@corivo/shared-types";

/**
 * Pull a human-readable message out of whatever an `invoke()` Promise
 * rejected with. Tauri 2 rejects with the serialized `TauriError`
 * tagged union (`{ kind, message, … }`); naive callers that do
 * `String(error)` get `"[object Object]"` and the user sees nothing
 * useful. CLAUDE.md long promised a `fromInvokeError` helper but
 * nobody actually wrote one — every existing call site has the same
 * latent bug, masked only by prefixing the noise with a "登出失败:"
 * style label. This is the canonical version.
 *
 * Accepts anything (mutations + Promises type their errors as
 * `unknown`) and falls back to `String(error)` only when none of the
 * common shapes apply.
 */
export function fromInvokeError(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (
    error !== null &&
    typeof error === "object" &&
    "message" in error &&
    typeof (error as { message: unknown }).message === "string"
  ) {
    return (error as { message: string }).message;
  }
  return String(error);
}

// --------------------------------------------------------------------------
// Cloud capabilities
//
// Single build-time probe used to decide which surfaces are reachable in
// this binary:
//   * `auth`              → render `/login` route, sidebar avatar
//   * `billing`           → show BillingDialog + "我的额度" entry
//   * `modelsDirectory`   → managed model picker; when false the Settings
//                            picker falls back to "用户自带 key"
//   * `connectors`        → Settings → Integrations tab
//   * `telemetry`         → Sentry init + business-event reporting
//   * `managedUpdater`    → fetch corivo policy + minVersion gate;
//                            when false the Tauri updater plugin is not
//                            registered and updater UI is hidden
//
// Open-source builds return all-false. Closed Corivo builds return true
// for every capability that initialised cleanly at boot (e.g.
// `modelsDirectory` is false when `app_data_dir` wasn't available, so
// the catalog couldn't be constructed).
//
// Cache through `useCapabilities()` — the result is build-time stable
// and there's no need to re-fetch.
// --------------------------------------------------------------------------

export type { Capabilities } from "@corivo/shared-types";

export async function getCapabilities(): Promise<Capabilities> {
  return invoke<Capabilities>("get_capabilities");
}

// --------------------------------------------------------------------------
// Config
// --------------------------------------------------------------------------

export async function getConfig(): Promise<Config> {
  return invoke<Config>("get_config");
}

export async function setConfig(config: Config): Promise<void> {
  return invoke<void>("set_config", { config });
}

// --------------------------------------------------------------------------
// Privacy filter (docs/privacy-filter-spec.md)
// --------------------------------------------------------------------------

/** Read the current privacy-filter preferences (enabled + per-category toggles). */
export async function getPrivacySettings(): Promise<PrivacySettings> {
  return invoke<PrivacySettings>("get_privacy_settings");
}

/**
 * Persist privacy-filter preferences and refresh the in-memory snapshot
 * the egress hook reads. `secret` is force-enabled server-side regardless
 * of the value sent — UI should keep that toggle disabled.
 */
export async function setPrivacySettings(
  settings: PrivacySettings,
): Promise<PrivacySettings> {
  return invoke<PrivacySettings>("set_privacy_settings", { settings });
}

/**
 * Clear the in-memory hash → spans LRU. Surfaced as a Settings button
 * ("clear privacy cache"); also called after a model upgrade so stale
 * spans don't outlive the model that produced them.
 */
export async function clearPrivacyCache(): Promise<void> {
  return invoke<void>("clear_privacy_cache");
}

/**
 * 模型整体状态。Settings 页打开时读一次,渲染:
 *   - downloaded=false → "需下载 X MB,加载后占 Y GB 内存"提醒
 *   - downloaded=true  → "模型已就绪"+ 删除/重下载按钮
 */
export async function getPrivacyModelStatus(): Promise<PrivacyModelStatus> {
  return invoke<PrivacyModelStatus>("get_privacy_model_status");
}

/**
 * 触发模型下载,进度通过 `onProgress` 流式回前端。
 * - 不会自动启用 settings.enabled,UI 在 Promise resolve 后再显式
 *   调 setPrivacySettings({enabled: true, ...})
 * - 模型已就绪时直接 resolve(),不会产生进度事件
 * - 失败:抛错,UI 应 toast 失败原因 + 保持 enabled=false
 */
export async function downloadPrivacyModel(
  onProgress: (event: ModelDownloadProgress) => void,
): Promise<void> {
  const channel = new Channel<ModelDownloadProgress>();
  channel.onmessage = onProgress;
  return invoke<void>("download_privacy_model", { progress: channel });
}

/**
 * 删除已下载的模型目录,并同步关闭 settings.enabled —— 避免开关 on
 * 但模型不存在的不一致状态。"重新下载" 也走这个:先 delete 再
 * download。
 */
export async function deletePrivacyModel(): Promise<void> {
  return invoke<void>("delete_privacy_model");
}

// --------------------------------------------------------------------------
// Capture pipeline
// --------------------------------------------------------------------------

export async function getCaptureStatus(): Promise<CaptureStatus> {
  return invoke<CaptureStatus>("capture_status");
}

export async function startCapture(): Promise<string> {
  return invoke<string>("capture_start");
}

export async function stopCapture(): Promise<void> {
  return invoke<void>("capture_stop");
}

export async function pauseCapture(
  seconds: number,
): Promise<CaptureStatus> {
  return invoke<CaptureStatus>("capture_pause", { seconds });
}

export async function resumeCapture(): Promise<CaptureStatus> {
  return invoke<CaptureStatus>("capture_resume");
}

export interface ExclusionEntry {
  bundle_id: string;
  source: "default" | "user";
}

export async function exclusionList(): Promise<ExclusionEntry[]> {
  return invoke<ExclusionEntry[]>("exclusion_list");
}

export async function exclusionAdd(bundleId: string): Promise<ExclusionEntry[]> {
  return invoke<ExclusionEntry[]>("exclusion_add", { bundleId });
}

export async function exclusionRemove(bundleId: string): Promise<ExclusionEntry[]> {
  return invoke<ExclusionEntry[]>("exclusion_remove", { bundleId });
}

export type { WebsiteExclusionEntry } from "@corivo/shared-types";

export async function websiteExclusionList(): Promise<WebsiteExclusionEntry[]> {
  return invoke<WebsiteExclusionEntry[]>("website_exclusion_list");
}

export async function websiteExclusionAdd(
  pattern: string,
): Promise<WebsiteExclusionEntry[]> {
  return invoke<WebsiteExclusionEntry[]>("website_exclusion_add", { pattern });
}

export async function websiteExclusionRemove(
  pattern: string,
): Promise<WebsiteExclusionEntry[]> {
  return invoke<WebsiteExclusionEntry[]>("website_exclusion_remove", { pattern });
}

// --------------------------------------------------------------------------
// Frames (spec §十一 frames_list / frame_detail / frames_search Phase 3+)
// --------------------------------------------------------------------------

export async function framesList(args: ListFramesArgs = {}): Promise<Frame[]> {
  return invoke<Frame[]>("frames_list", { args });
}

export async function frameDetail(id: string): Promise<Frame | null> {
  return invoke<Frame | null>("frame_detail", { id });
}

// --------------------------------------------------------------------------
// /ask (spec §十一)
// --------------------------------------------------------------------------

export async function chatThreadsList(): Promise<ChatThread[]> {
  return invoke<ChatThread[]>("chat_threads_list");
}

export async function chatThreadCreate(
  title?: string | null,
): Promise<ChatThread> {
  return invoke<ChatThread>("chat_thread_create", {
    title: title ?? null,
  });
}

export async function chatThreadDelete(id: string): Promise<void> {
  return invoke<void>("chat_thread_delete", { id });
}

/** v1300 sidebar lifecycle. `pinned: true` pins the thread under
 *  "置顶"; `false` removes the pin. */
export async function chatThreadSetPinned(
  id: string,
  pinned: boolean,
): Promise<void> {
  return invoke<void>("chat_thread_set_pinned", { id, pinned });
}

/** v1300 sidebar lifecycle. `archived: true` hides the thread from
 *  "最近" (surfaced only via "归档(N)"); `false` brings it back. */
export async function chatThreadSetArchived(
  id: string,
  archived: boolean,
): Promise<void> {
  return invoke<void>("chat_thread_set_archived", { id, archived });
}

export async function chatMessagesByThread(
  threadId: string,
): Promise<ChatMessage[]> {
  return invoke<ChatMessage[]>("chat_messages_by_thread", { threadId });
}

// v1200 three-step turn lifecycle (replaces chat_persist_turn). The
// previous single-call shape gated user-message durability on the
// assistant stream completing; the new shape persists user immediately,
// drops a streaming placeholder for assistant, and finalizes on stream
// end (success / error / cancel). See commands::chat module doc-comment
// for the full rationale.

/** Persist the user's message immediately on send. Returns the
 *  canonical row so the caller can correlate the streaming UI with the
 *  persisted id. Quick Ask attaches a focus context block; `/ask`
 *  passes `null`. */
export async function chatUserMessageCreate(args: {
  threadId: string;
  text: string;
  focusContext?: {
    frameId: string;
    summary: string;
    primaryText?: string | null;
    selection?: string | null;
  } | null;
}): Promise<ChatMessage> {
  return invoke<ChatMessage>("chat_user_message_create", {
    threadId: args.threadId,
    text: args.text,
    focusContext: args.focusContext
      ? {
          frame_id: args.focusContext.frameId,
          summary: args.focusContext.summary,
          primary_text: args.focusContext.primaryText ?? null,
          selection: args.focusContext.selection ?? null,
        }
      : null,
  });
}

/** Drop a `status='streaming'` assistant placeholder. Returns the row
 *  so the caller can hold the canonical id while the stream is alive. */
export async function chatAssistantMessageStart(args: {
  threadId: string;
}): Promise<ChatMessage> {
  return invoke<ChatMessage>("chat_assistant_message_start", {
    threadId: args.threadId,
  });
}

/** Terminal write for an assistant turn. Status moves out of
 *  `streaming` into one of `complete` / `error` / `cancelled`;
 *  the accumulated `content_blocks` array is committed in one shot. */
export async function chatAssistantMessageFinalize(args: {
  messageId: string;
  contentBlocks: ContentBlock[];
  citedFrameIds?: string[] | null;
  status: MessageStatus;
  errorMessage?: string | null;
  finishReason?: string | null;
  usage?: Usage | null;
}): Promise<ChatMessage> {
  return invoke<ChatMessage>("chat_assistant_message_finalize", {
    messageId: args.messageId,
    contentBlocks: args.contentBlocks,
    citedFrameIds: args.citedFrameIds ?? null,
    status: args.status,
    errorMessage: args.errorMessage ?? null,
    finishReason: args.finishReason ?? null,
    usage: args.usage ?? null,
  });
}

// --------------------------------------------------------------------------
// Closed-beta auth (see commands::auth + services::corivo_session)
// --------------------------------------------------------------------------

export async function authStatus(): Promise<AuthStatus> {
  return invoke<AuthStatus>("auth_status");
}

// --------------------------------------------------------------------------
// "Sign in with ChatGPT" (PKCE loopback against auth.openai.com).
// Activated by selecting the "Chatgpt" exec agent auth mode in Settings;
// the desktop persists tokens in Config.exec_agent.chatgpt and forwards
// them to the sidecar (which talks to chatgpt.com/backend-api/codex).
// --------------------------------------------------------------------------

import type { ChatgptAuthStatusView } from "@/lib/types";

export type { ChatgptAuthStatusView } from "@/lib/types";

/** Run the PKCE loopback flow against auth.openai.com. Long-running
 *  (~30s typical, capped at 5min) — UI should show a spinner.
 *  Resolves with the new status; rejects on user cancel, timeout, or
 *  ineligible ChatGPT plan. */
export async function chatgptAuthLogin(): Promise<ChatgptAuthStatusView> {
  return invoke<ChatgptAuthStatusView>("chatgpt_auth_login");
}

/** Wipe persisted ChatGPT credentials. Does NOT flip exec_agent.auth_mode —
 *  that's a separate user action via the Settings radio group. */
export async function chatgptAuthLogout(): Promise<ChatgptAuthStatusView> {
  return invoke<ChatgptAuthStatusView>("chatgpt_auth_logout");
}

export async function chatgptAuthStatus(): Promise<ChatgptAuthStatusView> {
  return invoke<ChatgptAuthStatusView>("chatgpt_auth_status");
}

/// Run the Google OAuth loopback flow and exchange the resulting ID
/// token for a Corivo session. The Rust side spawns a one-shot
/// 127.0.0.1 listener and opens the system browser to Google's
/// consent screen — the user clicks through with their already-logged-in
/// browser session and we resolve once the token-exchange + whitelist
/// check come back from the Corivo API.
///
/// Throws if:
///   - the OAuth client id isn't configured (CORIVO_GOOGLE_CLIENT_ID),
///   - the user denies consent in the browser,
///   - the user's Google email isn't in the beta whitelist (server 403),
///   - the loopback flow times out (5 min),
///   - the backend-owned token exchange fails.
export async function authLoginGoogle(): Promise<AuthStatus> {
  return invoke<AuthStatus>("auth_login_google");
}

/// Trigger sending a 6-digit OTP code to `email`. Returns the outcome —
/// the `rate_limited` branch is part of the success channel so the UI
/// can render an accurate cooldown rather than a generic error.
export async function authRequestEmailCode(
  email: string,
): Promise<RequestEmailCodeOutcome> {
  return invoke<RequestEmailCodeOutcome>("auth_request_email_code", { email });
}

/// Verify a 6-digit OTP code. The `ok` branch carries the same
/// AuthStatus shape `authLoginGoogle` returns — hand it straight to
/// `applyAuthStatus`. The `rejected` branch surfaces inline retry
/// hints (wrong code / expired / out of attempts) without going
/// through the failure pane. Network / 5xx errors still throw.
export async function authLoginEmail(
  email: string,
  code: string,
): Promise<EmailLoginOutcome> {
  return invoke<EmailLoginOutcome>("auth_login_email", { email, code });
}

export async function authLogout(): Promise<void> {
  return invoke<void>("auth_logout");
}

export async function authRefresh(): Promise<AuthStatus> {
  return invoke<AuthStatus>("auth_refresh");
}

// --------------------------------------------------------------------------
// Stripe top-up (BillingDialog). billingStartCheckout creates a checkout
// session through the cloud billing service and opens the URL in the user's
// default browser; payment confirmation arrives async server-side, so the
// UI must re-poll billingMe to surface the updated balance.
// --------------------------------------------------------------------------

export async function billingMe(): Promise<BillingMe> {
  return invoke<BillingMe>("billing_me");
}

export async function billingStartCheckout(amountUsd: number): Promise<void> {
  return invoke<void>("billing_start_checkout", { amountUsd });
}

// --------------------------------------------------------------------------
// Composio gateway. The desktop never touches Composio directly — these
// commands proxy through the cloud connector service, which holds the
// master API key server-side. The MCP tool surface is wired separately
// (see Rust `enabled_mcp_specs`); these are exclusively for the Settings
// UI (list connections / start OAuth / disconnect).
// --------------------------------------------------------------------------

export async function composioListConnections(): Promise<ComposioConnectionsResponse> {
  return invoke<ComposioConnectionsResponse>("composio_list_connections");
}

/** Begin an OAuth connection for the toolkit. Backend returns a Composio-
 *  hosted redirect URL; the Rust side opens it in the default browser as
 *  a side effect. Caller still gets the `connectionId` back to poll. */
export async function composioCreateConnectionLink(
  toolkitSlug: string,
): Promise<ComposioConnectionLink> {
  return invoke<ComposioConnectionLink>("composio_create_connection_link", {
    toolkitSlug,
  });
}

export async function composioDisconnect(connectionId: string): Promise<void> {
  return invoke<void>("composio_disconnect", { connectionId });
}

// --------------------------------------------------------------------------
// Model directory. The picker UI hits modelsGetAvailable on mount
// (instant disk read) and modelsRefresh on a "刷新" button press /
// behind the scenes after login. Both return the full ModelDirectory:
// top-level host + api_key are the per-user sub2api credentials, and
// `managed[]` is the list of aliases the admin has granted this user
// — see apps/api/src/routes/me.ts for the response contract.
//
// `ModelMeta` is exported above mainly so component prop types
// (e.g. ModelOption in exec-agent-section) can name it directly;
// callers fetching the catalog should consume `ModelDirectory`.
// --------------------------------------------------------------------------

export async function modelsGetAvailable(): Promise<ModelDirectory> {
  return invoke<ModelDirectory>("models_get_available");
}

export async function modelsRefresh(): Promise<ModelDirectory> {
  return invoke<ModelDirectory>("models_refresh");
}

// --------------------------------------------------------------------------
// Skill share — list host-installed skills the Settings UI can toggle.
// Mutation goes through setConfig(exec_agent.skill_share.enabled).
// --------------------------------------------------------------------------

export async function skillsListAvailable(): Promise<AvailableSkill[]> {
  return invoke<AvailableSkill[]>("skills_list_available");
}

// --------------------------------------------------------------------------
// Scheduled workflows (/workflows page, v1430).
//
// Definition + schedule + run history. The save/delete paths touch both
// $APPDATA/corivo/workflows/<slug>/WORKFLOW.md (filesystem) and the
// workflow_schedules / workflow_runs tables (SQLite). `runNow` enqueues
// onto the same BackgroundAgentScheduler the cron Ticker uses.
// --------------------------------------------------------------------------

export async function workflowsList(): Promise<WorkflowView[]> {
  return invoke<WorkflowView[]>("workflows_list");
}

export async function workflowsListRuns(
  args: { slug?: string; limit?: number } = {},
): Promise<WorkflowRun[]> {
  return invoke<WorkflowRun[]>("workflows_list_runs", args);
}

export async function workflowsGetRun(id: string): Promise<WorkflowRun | null> {
  return invoke<WorkflowRun | null>("workflows_get_run", { id });
}

/** Validate a trigger and return the next firing time, without writing
 *  anything. Used by the create/edit drawer to render "下次将在 …". */
export async function workflowsPreviewTrigger(
  trigger: Trigger,
): Promise<TriggerPreview> {
  return invoke<TriggerPreview>("workflows_preview_trigger", { trigger });
}

export async function workflowsSave(
  spec: WorkflowSaveSpec,
): Promise<WorkflowView> {
  return invoke<WorkflowView>("workflows_save", { spec });
}

export async function workflowsDelete(slug: string): Promise<void> {
  return invoke<void>("workflows_delete", { slug });
}

export async function workflowsSetEnabled(
  slug: string,
  enabled: boolean,
): Promise<WorkflowView> {
  return invoke<WorkflowView>("workflows_set_enabled", { slug, enabled });
}

/** Enqueue an immediate run. Returns the new `workflow_runs.id` so the
 *  caller can poll history for the resulting row. */
export async function workflowsRunNow(slug: string): Promise<string> {
  return invoke<string>("workflows_run_now", { slug });
}

/** Cancel a running or queued workflow by slug. Returns `true` when
 *  something was actually stopped. Backend's `on_dispatch_aborted`
 *  hook handles cleanup (records a failure run + emits
 *  `workflow:completed`) — frontend just needs to invalidate. */
export async function workflowsCancelRun(slug: string): Promise<boolean> {
  return invoke<boolean>("workflows_cancel_run", { slug });
}

/** Flip `workflow_runs.acknowledged_at` from NULL → now for one run.
 *  Used by the sidebar "Corivo 提议" section when the user opens a
 *  card, and by the history dialog auto-ack-on-open path. */
export async function workflowsAcknowledgeRun(runId: string): Promise<void> {
  return invoke<void>("workflows_acknowledge_run", { runId });
}

/** Total unread (`acknowledged_at IS NULL`) run count across every
 *  workflow. Feeds the sidebar "Corivo 提议" section's red dot. */
export async function workflowsUnreadCount(): Promise<number> {
  return invoke<number>("workflows_unread_count");
}

// --------------------------------------------------------------------------
// Connectors (Settings → Integrations)
//
// Generic over connector id — adding Notion / Slack / Calendar later
// means a new `packages/connector-<id>/manifest.json`, not new wrappers
// here.
// --------------------------------------------------------------------------

export async function connectorsList(): Promise<ConnectorSummary[]> {
  return invoke<ConnectorSummary[]>("connectors_list");
}

export async function connectorEnable(id: string): Promise<ConnectorSummary> {
  return invoke<ConnectorSummary>("connector_enable", { id });
}

export async function connectorDisable(id: string): Promise<void> {
  return invoke<void>("connector_disable", { id });
}

/** Long-running: opens the system browser and waits up to 5 minutes for
 *  the user to complete OAuth consent. UI should show a spinner. */
export async function connectorConnect(id: string): Promise<ConnectorSummary> {
  return invoke<ConnectorSummary>("connector_connect", { id });
}

/** Multi-connector connect on a single provider — opens the browser at
 *  most once with the union of every listed connector's manifest scopes,
 *  then binds and enables all of them on the resulting account. Used by
 *  the Settings → Integrations `ProviderCard` so checking three Google
 *  products and clicking Connect runs OAuth exactly once. All
 *  `connectorIds` must share `manifest.auth.provider == provider`. */
export async function connectorProviderConnect(
  provider: string,
  connectorIds: string[],
): Promise<ConnectorSummary[]> {
  return invoke<ConnectorSummary[]>("connector_provider_connect", {
    provider,
    connectorIds,
  });
}

/** Long-running like {@link connectorConnect}: spawns a sidecar bootstrap
 *  that opens the browser for the vendor MCP server's OAuth flow and
 *  caches the token under `$APPDATA/corivo-mcp-tokens/<id>/`. Used by
 *  `mcpServer`-shape connectors (Linear, future Notion/GitHub MCP). */
export async function connectorInstallMcp(id: string): Promise<ConnectorSummary> {
  return invoke<ConnectorSummary>("connector_install_mcp", { id });
}

export async function connectorDisconnect(id: string): Promise<ConnectorSummary> {
  return invoke<ConnectorSummary>("connector_disconnect", { id });
}

// --------------------------------------------------------------------------
// Chat — backed by the corivo-agent sidecar (see exec_agent below).
// Both `/ask` and Quick Ask go through `execAgentSend`. Quick Ask
// passes a `focusContext`; `/ask` doesn't.
// --------------------------------------------------------------------------

export interface ExecAgentFocusContext {
  /** ULID of the frame this message is anchored to. Each Quick Ask
   *  message carries its own frame so conversation history preserves
   *  "what was on screen when this question was asked" (CO-3). */
  frameId: string;
  /** One-line description of what window the user is looking at. */
  summary: string;
  /** Truncated AX/OCR text from the focused window — surfaced to claude
   *  as "可见文本片段". The backend caps to 800 chars. */
  primaryText: string;
  /** User's current selection from AX. When non-empty the backend
   *  surfaces it as a dedicated "用户高亮选区" block in the prompt. */
  selection?: string | null;
}

export async function execAgentSend(
  threadId: string,
  content: string,
  stream: Channel<string>,
  /** Id of the streaming assistant row created by
   *  `chatAssistantMessageStart`. Required so the backend can stamp
   *  `model_used` once it resolves the alias for this turn. */
  assistantMessageId: string,
  focusContext?: ExecAgentFocusContext,
): Promise<void> {
  return invoke<void>("exec_agent_send", {
    threadId,
    content,
    stream,
    assistantMessageId,
    focusContext: focusContext
      ? {
          frame_id: focusContext.frameId,
          summary: focusContext.summary,
          primary_text: focusContext.primaryText,
          selection: focusContext.selection ?? null,
        }
      : null,
  });
}

/** Switch the user's active model alias. Blocks until the backend
 *  confirms the gateway-side group flip; returns the refreshed
 *  directory with `active_alias` updated. */
export async function modelsSetActiveModel(
  alias: string,
): Promise<ModelDirectory> {
  return invoke<ModelDirectory>("models_set_active_model", { args: { alias } });
}

export async function execAgentPermissionReply(args: {
  requestId: string;
  allow: boolean;
  message?: string | null;
}): Promise<void> {
  return invoke<void>("exec_agent_permission_reply", {
    requestId: args.requestId,
    allow: args.allow,
    message: args.message ?? null,
  });
}

/**
 * Cancel an in-flight `exec_agent_send` for `threadId`. Fire-and-
 * forget — Rust always returns Ok even if no turn is running, so this
 * is safe to call on a Stop button without checking state. The runner
 * reacts by killing the corivo-agent child; the stream channel then
 * delivers a final `finish` event with `reason: "cancelled"` and
 * `useChatStream` flips `isStreaming` off via its existing handler.
 */
export async function execAgentCancel(threadId: string): Promise<void> {
  return invoke<void>("exec_agent_cancel", { threadId });
}

// --------------------------------------------------------------------------
// Quick Ask (spec §十一)
// --------------------------------------------------------------------------

export async function quickAskSummon(): Promise<void> {
  return invoke<void>("quick_ask_summon");
}

export async function quickAskHide(): Promise<void> {
  return invoke<void>("quick_ask_hide");
}

export async function quickAskSetHeight(height: number): Promise<void> {
  return invoke<void>("quick_ask_set_height", { height });
}

export async function quickAskLogDisplay(
  app: string | null,
  source: string,
): Promise<void> {
  return invoke<void>("quick_ask_log_display", { app, source });
}

export async function quickAskCaptureFocus(): Promise<FocusContext> {
  return invoke<FocusContext>("quick_ask_capture_focus");
}

/// Resolve a macOS bundle id to a PNG data URL (32×32, base64) suitable
/// for an `<img src=...>` in the Quick Ask FocusCard. Returns null when
/// the bundle id can't be resolved or the host is non-macOS. Results
/// (including null) are cached in the Rust process forever — repeated
/// calls for the same id are essentially free.
export async function getAppIcon(bundleId: string): Promise<string | null> {
  return invoke<string | null>("get_app_icon", { bundleId });
}

/// Hide the Quick Ask overlay, raise the main window, and route `/ask`
/// to the given thread. Backs the "在 App 中查看" button on the panel.
export async function quickAskOpenInApp(threadId: string): Promise<void> {
  return invoke<void>("quick_ask_open_in_app", { threadId });
}

export async function getHotkeyStatus(): Promise<HotkeyStatus> {
  return invoke<HotkeyStatus>("hotkey_status");
}

// --------------------------------------------------------------------------
// Settings + onboarding (unchanged surface)
// --------------------------------------------------------------------------

export async function setAutostartEnabled(enabled: boolean): Promise<void> {
  return invoke<void>("set_autostart_enabled", { enabled });
}

export async function getAutostartEnabled(): Promise<boolean> {
  return invoke<boolean>("get_autostart_enabled");
}

export async function getSystemInfo(): Promise<SystemInfo> {
  return invoke<SystemInfo>("get_system_info");
}

export async function restartAsAdministrator(): Promise<void> {
  return invoke<void>("restart_as_administrator");
}

export async function clearAllScreenshots(): Promise<number> {
  return invoke<number>("clear_all_screenshots");
}

export interface HardDeleteSummary {
  frames_before: number;
  sessions_deleted: number;
  screenshots_deleted: number;
}

export async function dataHardDelete(): Promise<HardDeleteSummary> {
  return invoke<HardDeleteSummary>("data_hard_delete");
}

export type { DeleteRangeReport } from "@corivo/shared-types";

/**
 * Drop every frame captured in the trailing `seconds_back` seconds plus
 * their on-disk screenshots. Backs the "delete last 5 min / 15 min /
 * custom…" entries in the context-awareness popover. The Rust side
 * clamps `secondsBack` at one year — anything larger is treated as a
 * year. Unlike `dataHardDelete` this does not stop the capture pipeline.
 */
export async function dataDeleteRange(
  secondsBack: number,
): Promise<DeleteRangeReport> {
  return invoke<DeleteRangeReport>("data_delete_range", { secondsBack });
}

export async function checkScreenRecordingPermission(): Promise<boolean> {
  return invoke<boolean>("check_screen_recording_permission");
}

export async function requestScreenRecordingPermission(): Promise<boolean> {
  return invoke<boolean>("request_screen_recording_permission");
}

export async function openSystemSettingsPrivacy(): Promise<void> {
  return invoke<void>("open_system_settings_privacy");
}

export async function checkAxPermission(): Promise<boolean> {
  return invoke<boolean>("check_ax_permission");
}

export async function openAxSettings(): Promise<void> {
  return invoke<void>("open_ax_settings");
}

export async function openCapturesDirectory(): Promise<void> {
  return invoke<void>("open_captures_directory");
}

export async function getDataPaths(): Promise<DataPaths> {
  return invoke<DataPaths>("get_data_paths");
}

export async function getOnboardingState(): Promise<OnboardingState> {
  return invoke<OnboardingState>("get_onboarding_state");
}

export async function saveOnboardingStep(step: string): Promise<void> {
  return invoke<void>("save_onboarding_step", { step });
}

export async function markOnboardingCompleted(): Promise<void> {
  return invoke<void>("mark_onboarding_completed");
}

/**
 * Flip `Config.app.first_quick_ask_done` to `true`. Idempotent on the
 * Rust side. Quick Ask calls this on the user's first sent turn during
 * the Try-It onboarding step; after this fires, the suggestion chips
 * stop appearing forever.
 */
export async function markFirstQuickAskDone(): Promise<void> {
  return invoke<void>("mark_first_quick_ask_done");
}

export async function resetOnboarding(): Promise<void> {
  return invoke<void>("reset_onboarding");
}

// --------------------------------------------------------------------------
// Developer mode (Settings → Developer; unlocked by triple-clicking the
// dialog title). Production-build DevTools requires the `devtools` feature
// on `tauri` in Cargo.toml — `dev_open_devtools` is a no-op without it.
// --------------------------------------------------------------------------

export type DevWindowLabel = "main" | "quick-ask" | "notification-overlay";

export async function devOpenDevtools(label: DevWindowLabel): Promise<void> {
  return invoke<void>("dev_open_devtools", { label });
}

export async function devRevealDataDir(): Promise<void> {
  return invoke<void>("dev_reveal_data_dir");
}

// --------------------------------------------------------------------------
// Frontend → Rust tracing log bridge
// --------------------------------------------------------------------------

export type FrontendLogLevel = "debug" | "info" | "warn" | "error";

// Fire-and-forget: 日志桥失败不能让业务流程也跟着挂。`invoke` 在 jsdom/node 测试
// 环境下可能因为没有 `__TAURI_INTERNALS__` 而同步抛错，所以外面再裹一层 try。
export function logFrontend(
  level: FrontendLogLevel,
  target: string,
  message: string,
  fields?: Record<string, unknown>,
): void {
  try {
    void invoke<void>("log_frontend", {
      level,
      target,
      message,
      fields,
    }).catch(() => {});
  } catch {
    // ignore
  }
}

// 业务埋点 → Sentry message event。同样 fire-and-forget：埋点失败不能
// 影响用户实际操作（点击付费、登录之类）。在 sentry 后台以独立 Info
// event 出现，不污染 error rate。
export function trackEvent(
  name: string,
  properties?: Record<string, unknown>,
): void {
  try {
    void invoke<void>("track_event", {
      args: { name, properties },
    }).catch(() => {});
  } catch {
    // ignore
  }
}

// --------------------------------------------------------------------------
// Memory system (notes — memory-system-spec §3 / §10.5)
// --------------------------------------------------------------------------

export interface ListNotesArgs {
  scope?: NoteScope;
  scopeRef?: string;
  status?: NoteStatus;
  sourceType?: NoteSourceType;
  limit?: number;
}

export async function listNotes(args: ListNotesArgs = {}): Promise<Note[]> {
  return invoke<Note[]>("list_notes", {
    scope: args.scope ?? null,
    scopeRef: args.scopeRef ?? null,
    status: args.status ?? null,
    sourceType: args.sourceType ?? null,
    limit: args.limit ?? null,
  });
}

export async function getNote(id: string): Promise<Note | null> {
  return invoke<Note | null>("get_note", { id });
}

export async function createNote(args: {
  content: string;
  scope?: NoteScope;
  scopeRef?: string;
}): Promise<Note> {
  return invoke<Note>("create_note", {
    content: args.content,
    scope: args.scope ?? null,
    scopeRef: args.scopeRef ?? null,
  });
}

export async function updateNote(args: {
  id: string;
  content?: string;
  status?: NoteStatus;
  confidence?: number;
}): Promise<Note> {
  return invoke<Note>("update_note", {
    id: args.id,
    content: args.content ?? null,
    status: args.status ?? null,
    confidence: args.confidence ?? null,
  });
}

export async function deleteNote(id: string): Promise<void> {
  return invoke<void>("delete_note", { id });
}

// --------------------------------------------------------------------------
// Persona (memory-system-spec §4)
// --------------------------------------------------------------------------

export async function getAutoPersona(): Promise<string> {
  return invoke<string>("get_auto_persona");
}

export async function regenerateAutoPersona(): Promise<void> {
  return invoke<void>("regenerate_auto_persona");
}

export async function openBackgroundTaskLogsDir(): Promise<void> {
  return invoke<void>("open_background_task_logs_dir");
}

// --------------------------------------------------------------------------
// Stub kept for old call sites — dropped in Phase 6 cleanup pass.
// --------------------------------------------------------------------------

export type { TauriError, StorageStats };
