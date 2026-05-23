import { Channel } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useCallback, useEffect, useMemo, useRef } from "react";
import {
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";

import { useTranslation } from "@/i18n";
import {
  chatAssistantMessageFinalize,
  chatAssistantMessageStart,
  chatMessagesByThread,
  chatThreadCreate,
  chatThreadDelete,
  chatThreadSetArchived,
  chatThreadSetPinned,
  chatThreadsList,
  chatUserMessageCreate,
  execAgentCancel,
  execAgentSend,
  type ExecAgentFocusContext,
} from "@/lib/tauri";
import { useActiveThreadStore } from "@/stores/active-thread-store";
import {
  selectThreadStream,
  useStreamingStore,
} from "@/stores/streaming-store";
import type { ChatMessage, ChatStreamEvent } from "@/lib/types";
import type {
  ChatThreadsChanged,
  ContentBlock,
  MessageStatus,
} from "@corivo/shared-types";

export const CHAT_QUERY_KEY = ["chat"] as const;

/// Tauri event name backing cross-window thread-list sync. Mirrors
/// `ChatThreadsChanged::EVENT` in `apps/desktop/src-tauri/src/domain/chat.rs`.
const CHAT_THREADS_CHANGED_EVENT = "chat:threads-changed";

/// Frontend-only Tauri event used to mirror an in-flight chat stream
/// across webview windows. The sender (the window that opened the
/// `Channel<String>` to `exec_agent_send`) re-emits every stream event
/// here so the other window can render the same live UI in real time.
/// `originLabel` is the sender's webview label — receivers ignore their
/// own emits so the originator doesn't double-process events.
const CHAT_STREAM_MIRROR_EVENT = "chat:stream-mirror";

interface ChatStreamMirrorPayload {
  originLabel: string;
  threadId: string;
  assistantId: string;
  event: ChatStreamEvent;
}

/**
 * Cross-window chat cache sync. Mount once per webview at the root —
 * AppBoot for the main window, QuickAskWindow for the overlay.
 *
 * Reacts to backend `chat:threads-changed` broadcasts (fired on user
 * message create, assistant finalize, thread create / rename / pin /
 * archive / delete) by invalidating two caches:
 *   - the threads list, so the sidebar reorders / shows new rows;
 *   - the affected thread's message list, so the conversation view
 *     refetches when a write from another window (Quick Ask in
 *     particular) lands in a thread this window is currently viewing.
 */
export function useChatThreadsSync() {
  const qc = useQueryClient();
  useEffect(() => {
    const unlistenPromise = listen<ChatThreadsChanged>(
      CHAT_THREADS_CHANGED_EVENT,
      (event) => {
        void qc.invalidateQueries({
          queryKey: [...CHAT_QUERY_KEY, "threads"],
        });
        const threadId = event.payload.thread_id;
        if (threadId) {
          void qc.invalidateQueries({
            queryKey: [...CHAT_QUERY_KEY, "messages", threadId],
          });
        }
      },
    );
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [qc]);
}

/**
 * Cross-window streaming mirror. Mount once per webview at the root —
 * AppBoot for the main window, QuickAskWindow for the overlay.
 *
 * Reacts to frontend `chat:stream-mirror` events emitted by whichever
 * window owns the in-flight `Channel<String>` to `exec_agent_send`. The
 * sender re-emits every stream event so the other window can drive the
 * same `useStreamingStore` bucket and render the live response in real
 * time — text deltas, tool calls, citations, the lot — without each
 * window having to own a Channel of its own.
 *
 * Lazy-creates a streaming bucket the first time it sees an event for
 * a thread it doesn't track. Skips its own emits via the originLabel
 * round-trip so the sender doesn't double-process events it already
 * handled locally in `channel.onmessage`.
 *
 * The bucket is intentionally NOT cleared when the stream finishes:
 * finalize fires `chat:threads-changed`, the `useChatThreadsSync`
 * listener invalidates the messages cache, the persisted assistant
 * row is refetched, and the merge dedup in `useChatStream` hides the
 * live row by id. The stale bucket is overwritten by the next
 * `startStream` on that thread. Cleaner than racing the two events
 * for the right moment to clear.
 */
export function useChatStreamMirror() {
  const { t } = useTranslation();
  const fallbackRef = useRef(t.ask.streamErrorFallback);
  fallbackRef.current = t.ask.streamErrorFallback;
  useEffect(() => {
    const myLabel = getCurrentWebviewWindow().label;
    const unlistenPromise = listen<ChatStreamMirrorPayload>(
      CHAT_STREAM_MIRROR_EVENT,
      (event) => {
        const payload = event.payload;
        if (payload.originLabel === myLabel) return;
        const store = useStreamingStore.getState();
        const bucket = store.byThreadId[payload.threadId];
        const hasAssistant = bucket?.liveMessages.some(
          (m) => m.id === payload.assistantId,
        );
        if (!hasAssistant) {
          const liveAssistant: LiveChatMessage = {
            id: payload.assistantId,
            role: "assistant",
            content: "",
            toolEvents: [],
            segments: [],
            citedFrameIds: [],
            isStreaming: true,
          };
          store.startStream(payload.threadId, liveAssistant);
        }
        handleStreamEvent(
          payload.threadId,
          payload.assistantId,
          payload.event,
          fallbackRef.current,
        );
      },
    );
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);
}

export function useChatThreads() {
  return useQuery({
    queryKey: [...CHAT_QUERY_KEY, "threads"],
    queryFn: chatThreadsList,
  });
}

export function useCreateChatThread() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (title?: string) => chatThreadCreate(title ?? null),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: [...CHAT_QUERY_KEY, "threads"] });
    },
  });
}

export function useDeleteChatThread() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => chatThreadDelete(id),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: [...CHAT_QUERY_KEY, "threads"] });
    },
  });
}

/** v1300 sidebar lifecycle. Toggle a thread's pinned state. The
 *  backend emits `chat:threads-changed` so the sidebar in every
 *  webview re-renders, but we also locally invalidate so this
 *  window's cache flips immediately without waiting for the event. */
export function usePinChatThread() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, pinned }: { id: string; pinned: boolean }) =>
      chatThreadSetPinned(id, pinned),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: [...CHAT_QUERY_KEY, "threads"] });
    },
  });
}

/** v1300 sidebar lifecycle. Toggle a thread's archived state. */
export function useArchiveChatThread() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, archived }: { id: string; archived: boolean }) =>
      chatThreadSetArchived(id, archived),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: [...CHAT_QUERY_KEY, "threads"] });
    },
  });
}

export function usePersistedChatMessages(threadId: string | null) {
  return useQuery({
    queryKey: [...CHAT_QUERY_KEY, "messages", threadId],
    queryFn: () =>
      threadId ? chatMessagesByThread(threadId) : Promise.resolve([]),
    enabled: threadId !== null,
  });
}

/** A message in the live UI — extends the persisted shape with
 *  streaming-time fields the chat agent emits before the assistant
 *  turn finalizes in the DB. */
export interface LiveChatMessage {
  id: string;
  role: "user" | "assistant" | "system";
  /** Accumulated text — kept for display + as the source for the
   *  finalized `content_blocks` array's text rollup. */
  content: string;
  /** Tool calls + their results, in the order the agent emitted them.
   *  Flat list, used to build the `tool_use` / `tool_result` blocks
   *  on finalize. */
  toolEvents: LiveToolEvent[];
  /** Time-ordered timeline of segments — text / thinking / tool
   *  clusters — for rendering. Persisted rounds derive these from
   *  `content_blocks` at load time. */
  segments: AssistantSegment[];
  citedFrameIds: string[];
  /** Human label of the frame this user message is anchored to (CO-3).
   *  Only set on live user messages where we still have the in-memory
   *  FocusContext; persisted rounds re-render from `citedFrameIds`
   *  alone with a generic hint. */
  userFrameLabel?: string;
  /** True until the agent emits `finish`. */
  isStreaming: boolean;
  error?: string;
  /** Transient hint shown alongside the thinking dots. */
  status?: string;
  /** Model directory alias that drove this assistant turn — surfaces
   *  as a "via <alias>" subscript under the bubble. `null` on user
   *  rows, BYOK assistant rows, and legacy rows. The live store
   *  leaves it `null` while streaming; finalize backfills it via
   *  `persistedToLive` once the row lands. */
  modelUsed?: string | null;
}

export type AssistantSegment =
  | { kind: "text"; text: string }
  | { kind: "thinking"; text: string }
  | { kind: "tools"; events: LiveToolEvent[] };

export interface LiveToolEvent {
  id: string;
  name: string;
  args: Record<string, unknown>;
  /** Arrives later via tool-result; null until then. */
  result: unknown | null;
}

interface UseChatStreamReturn {
  messages: LiveChatMessage[];
  isStreaming: boolean;
  error: string | null;
  sendMessage: (
    content: string,
    focus?: ExecAgentFocusContext,
    frameLabel?: string,
  ) => Promise<void>;
  /** Cancel the in-flight stream, if any. Fires `exec_agent_cancel`
   *  which kills the corivo-agent child; the runner emits a final
   *  `finish` event with `reason: "cancelled"` and the existing
   *  finalize path persists the partial blocks with status='cancelled'.
   *  No-op when nothing is streaming. */
  cancel: () => Promise<void>;
  reset: () => void;
}

export interface UseChatStreamOptions {
  /** If provided, sendMessage may create a thread on demand when
   *  `threadId` is null. */
  createThreadIfMissing?: () => Promise<string>;
  /** Notified synchronously after `createThreadIfMissing` resolves. */
  onThreadCreated?: (threadId: string) => void;
}

/**
 * Single chat stream — backed by `exec_agent_send` (corivo-agent
 * sidecar). v1200 lifecycle:
 *
 *   1. `chatUserMessageCreate` — user message lands in DB immediately
 *   2. `chatAssistantMessageStart` — placeholder row (`status='streaming'`)
 *   3. stream events drive the live UI (`liveMessages`) + accumulate into
 *      a final `ContentBlock[]`
 *   4. `chatAssistantMessageFinalize` — terminal write (success / error /
 *      cancel)
 *
 * Crucially: a stream failure (network, agent crash, user cancel) cannot
 * cause the user's prompt to disappear. Step 1 is committed before the
 * agent even spawns.
 *
 * The live `LiveChatMessage[]` / `isStreaming` / `error` triple for the
 * current thread is read from `useStreamingStore` rather than held in
 * local state, so switching threads mid-stream no longer drops the
 * in-flight buffer. The Channel `onmessage` callback writes to the
 * per-thread bucket directly, which means the background turn keeps
 * filling its bucket regardless of which thread the UI is viewing.
 */
export function useChatStream(
  threadId: string | null,
  options: UseChatStreamOptions = {},
): UseChatStreamReturn {
  const { t } = useTranslation();
  const persisted = usePersistedChatMessages(threadId);
  const qc = useQueryClient();
  const live = useStreamingStore((s) => selectThreadStream(s, threadId));
  const { liveMessages, isStreaming, error } = live;

  const lastThreadId = useRef<string | null>(threadId);
  if (threadId) lastThreadId.current = threadId;
  const optionsRef = useRef(options);
  optionsRef.current = options;
  // Per-thread send guard (CO-61). The store-level `isStreaming` only
  // flips on after `startStream` has opened a bucket — there's a window
  // before that where double-submitting the same prompt would race.
  // This set gates `sendMessage` on a per-thread basis (with a
  // "__draft__" sentinel for the type-to-create flow), so a turn
  // already streaming in thread A no longer blocks a fresh send from
  // thread B or a new draft.
  const sendingRef = useRef<Set<string>>(new Set());
  const DRAFT_GUARD_KEY = "__draft__";

  const messages = useMemo<LiveChatMessage[]>(() => {
    const persistedAsLive = (persisted.data ?? []).map(persistedToLive);
    // Dedup by id so the cross-window stream mirror doesn't double-
    // render the assistant row at finalize. The originator clears its
    // own live bucket; the mirroring window leaves a stale entry in
    // `useStreamingStore` and relies on this filter to hide it once
    // the finalized assistant lands in `persisted`.
    const persistedIds = new Set(persistedAsLive.map((m) => m.id));
    const filteredLive = liveMessages.filter((m) => !persistedIds.has(m.id));
    return [...persistedAsLive, ...filteredLive];
  }, [persisted.data, liveMessages]);

  const sendMessage = useCallback(
    async (
      content: string,
      focus?: ExecAgentFocusContext,
      _frameLabel?: string,
    ) => {
      const guardKey = threadId ?? DRAFT_GUARD_KEY;
      if (sendingRef.current.has(guardKey)) return;
      sendingRef.current.add(guardKey);
      try {
        const store = useStreamingStore.getState();

        let effectiveId = threadId;
        if (!effectiveId) {
          const create = optionsRef.current.createThreadIfMissing;
          if (!create) {
            console.error(t.ask.noActiveThread);
            return;
          }
          try {
            effectiveId = await create();
          } catch (e) {
            console.error("createThreadIfMissing failed", e);
            return;
          }
          lastThreadId.current = effectiveId;
          optionsRef.current.onThreadCreated?.(effectiveId);
        }

        // Step 1: persist the user's message IMMEDIATELY. From this
        // point on the prompt is durable regardless of what happens
        // downstream — the legacy "lose user message on agent crash"
        // bug is fixed structurally here.
        try {
          await chatUserMessageCreate({
            threadId: effectiveId,
            text: content,
            focusContext: focus
              ? {
                  frameId: focus.frameId,
                  summary: focus.summary,
                  primaryText: focus.primaryText,
                  selection: focus.selection,
                }
              : null,
          });
        } catch (e) {
          store.setError(effectiveId, `保存提问失败: ${String(e)}`);
          return;
        }
        // Surface the user row + reorder threads list right away.
        await qc.invalidateQueries({
          queryKey: [...CHAT_QUERY_KEY, "messages", effectiveId],
        });
        await qc.invalidateQueries({
          queryKey: [...CHAT_QUERY_KEY, "threads"],
        });

        // Step 2: drop the assistant placeholder. We need the canonical
        // id so finalize knows which row to update.
        let assistantRow: ChatMessage;
        try {
          assistantRow = await chatAssistantMessageStart({
            threadId: effectiveId,
          });
        } catch (e) {
          store.setError(effectiveId, `创建 assistant 占位失败: ${String(e)}`);
          return;
        }

        // Step 3: open the per-thread streaming bucket. From here on,
        // `channel.onmessage` writes into the global store keyed on
        // `effectiveId`, so the data survives any UI thread switch.
        const liveAssistant: LiveChatMessage = {
          id: assistantRow.id,
          role: "assistant",
          content: "",
          toolEvents: [],
          segments: [],
          citedFrameIds: [],
          isStreaming: true,
        };
        store.startStream(effectiveId, liveAssistant);

        const accumulator = new BlockAccumulator();
        const channel = new Channel<string>();
        const streamErrorFallback = t.ask.streamErrorFallback;
        const bucketThreadId = effectiveId;
        // Snapshot once: a webview's label doesn't change for the
        // lifetime of the window, so there's no need to re-resolve it
        // for every chunk.
        const originLabel = getCurrentWebviewWindow().label;
        channel.onmessage = (line) => {
          for (const event of parseEvents(line)) {
            handleStreamEvent(
              bucketThreadId,
              assistantRow.id,
              event,
              streamErrorFallback,
            );
            accumulator.consume(event);
            // Mirror to the other webview so it can render the same
            // live UI without owning a Channel. The other side's
            // `useChatStreamMirror` lazy-creates a bucket and replays
            // each event through the same `handleStreamEvent` path.
            // Sender ignores its own emits via `originLabel`.
            void emit(CHAT_STREAM_MIRROR_EVENT, {
              originLabel,
              threadId: bucketThreadId,
              assistantId: assistantRow.id,
              event,
            } satisfies ChatStreamMirrorPayload).catch(() => undefined);
          }
        };

        let runError: string | undefined;
        try {
          await execAgentSend(
            effectiveId,
            content,
            channel,
            assistantRow.id,
            focus,
          );
        } catch (e) {
          runError = String(e);
          store.setError(effectiveId, runError);
          store.updateLiveMessages(effectiveId, (prev) =>
            prev.map((m) =>
              m.id === assistantRow.id
                ? { ...m, isStreaming: false, error: runError }
                : m,
            ),
          );
        } finally {
          store.setStreaming(effectiveId, false);
        }

        // Step 4: finalize. Status reflects the actual stream outcome.
        // We finalize even on error so partial blocks land on disk and
        // the row stops being `streaming`.
        const status: MessageStatus = runError
          ? "error"
          : accumulator.cancelled
            ? "cancelled"
            : accumulator.errorFromStream
              ? "error"
              : "complete";
        const errorMessage =
          runError ?? accumulator.errorFromStream ?? null;
        try {
          await chatAssistantMessageFinalize({
            messageId: assistantRow.id,
            contentBlocks: accumulator.toBlocks(),
            citedFrameIds: accumulator.citedFrameIds,
            status,
            errorMessage,
            finishReason: accumulator.finishReason,
            // Usage isn't surfaced over the current wire schema yet
            // (sidecar emits it on `agent_end` but the Rust forwarder
            // doesn't propagate). When wired, populate from a separate
            // event handler.
            usage: null,
          });
        } catch (e) {
          // Finalize failed — leave the placeholder row as `streaming`;
          // the boot orphan-cleanup will mark it cancelled on next start.
          // The user-visible state is governed by the live bucket (which
          // already reflects the error).
          console.error("chat_assistant_message_finalize failed", e);
        }
        await qc.invalidateQueries({
          queryKey: [...CHAT_QUERY_KEY, "messages", effectiveId],
        });
        await qc.invalidateQueries({
          queryKey: [...CHAT_QUERY_KEY, "threads"],
        });
        // Drop the in-memory live shape — the persisted row now owns
        // the canonical render.
        store.clearStream(effectiveId);
        // CO-62: if the user navigated away while this turn was
        // running, flag the thread as having unviewed activity so the
        // sidebar can surface the dot. Cancellations come from the
        // user themselves, so we don't flag those.
        if (!accumulator.cancelled) {
          const activeNow = useActiveThreadStore.getState().activeId;
          if (activeNow !== effectiveId) {
            useActiveThreadStore.getState().markUnread(effectiveId);
          }
        }
      } finally {
        sendingRef.current.delete(guardKey);
      }
    },
    [threadId, qc, t],
  );

  const reset = useCallback(() => {
    const tid = lastThreadId.current;
    if (tid) useStreamingStore.getState().clearStream(tid);
  }, []);

  const cancel = useCallback(async () => {
    const tid = lastThreadId.current;
    if (!tid) return;
    const bucket = useStreamingStore.getState().byThreadId[tid];
    if (!bucket?.isStreaming) return;
    try {
      await execAgentCancel(tid);
    } catch (e) {
      // Best-effort: a missing in-flight turn is fine, the runner's
      // cancel command tolerates it. Log and continue — the stream
      // will resolve naturally if it was already winding down.
      console.warn("execAgentCancel failed", e);
    }
  }, []);

  return {
    messages,
    isStreaming,
    error,
    sendMessage,
    cancel,
    reset,
  };
}

function parseEvents(raw: string): ChatStreamEvent[] {
  return raw
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      try {
        return JSON.parse(line) as ChatStreamEvent;
      } catch {
        return null;
      }
    })
    .filter((e): e is ChatStreamEvent => e !== null);
}

/** Drives the live in-memory `LiveChatMessage` shape from streaming
 *  events. Same logic as before — text / tool clusters / cited frames /
 *  status / finish / error. Writes go to the per-thread bucket in the
 *  global streaming store so the data survives UI thread switches. */
function handleStreamEvent(
  threadId: string,
  assistantId: string,
  event: ChatStreamEvent,
  streamErrorFallback: string,
) {
  const store = useStreamingStore.getState();
  store.updateLiveMessages(threadId, (prev) =>
    prev.map((m) => {
      if (m.id !== assistantId) return m;
      switch (event.type) {
        case "text-delta":
          return {
            ...m,
            content: m.content + event.text,
            segments: appendText(m.segments, event.text),
            status: undefined,
          };
        case "tool-call": {
          const newEvent: LiveToolEvent = {
            id: event.id,
            name: event.name,
            args: event.args,
            result: null,
          };
          return {
            ...m,
            toolEvents: [...m.toolEvents, newEvent],
            segments: appendToolCall(m.segments, newEvent),
            status: undefined,
          };
        }
        case "tool-result":
          return {
            ...m,
            toolEvents: m.toolEvents.map((t) =>
              t.id === event.id ? { ...t, result: event.result } : t,
            ),
            segments: applyToolResult(m.segments, event.id, event.result),
          };
        case "cited-frames": {
          const next = new Set([...m.citedFrameIds, ...event.frame_ids]);
          return { ...m, citedFrameIds: Array.from(next) };
        }
        case "status":
          return { ...m, status: event.detail ?? undefined };
        case "finish":
          return {
            ...m,
            isStreaming: false,
            status: undefined,
            error:
              event.reason === "error"
                ? (m.error ?? streamErrorFallback)
                : m.error,
          };
        case "error":
          return {
            ...m,
            isStreaming: false,
            status: undefined,
            error: event.message,
          };
      }
    }),
  );
  // Reset the bucket-level `isStreaming` on terminal events. The sidebar
  // ActivityBadge reads bucket.isStreaming (not the per-message flag), so
  // mirror windows — which never run the originator's `finally`
  // setStreaming(false) — would otherwise keep pulsing forever after the
  // turn finishes.
  if (event.type === "finish" || event.type === "error") {
    store.setStreaming(threadId, false);
  }
  if (event.type === "error") {
    store.setError(threadId, event.message);
  }
}

function appendText(
  segments: AssistantSegment[],
  text: string,
): AssistantSegment[] {
  if (!text) return segments;
  const last = segments[segments.length - 1];
  if (last && last.kind === "text") {
    return [
      ...segments.slice(0, -1),
      { kind: "text", text: last.text + text },
    ];
  }
  return [...segments, { kind: "text", text }];
}

function appendToolCall(
  segments: AssistantSegment[],
  event: LiveToolEvent,
): AssistantSegment[] {
  const last = segments[segments.length - 1];
  if (last && last.kind === "tools") {
    return [
      ...segments.slice(0, -1),
      { kind: "tools", events: [...last.events, event] },
    ];
  }
  return [...segments, { kind: "tools", events: [event] }];
}

function applyToolResult(
  segments: AssistantSegment[],
  eventId: string,
  result: unknown,
): AssistantSegment[] {
  return segments.map((seg) => {
    if (seg.kind !== "tools") return seg;
    const updated = seg.events.map((e) =>
      e.id === eventId ? { ...e, result } : e,
    );
    return { kind: "tools", events: updated };
  });
}

/**
 * Build a `ContentBlock[]` from the agent's streaming events. Used at
 * finalize time to commit the canonical block array to the DB.
 *
 * Block boundaries follow event types:
 *   - text-delta           → grow the current text block (or open one)
 *   - cited-frames         → emit a frame_citation block
 *   - tool-call            → emit a tool_use block
 *   - tool-result          → emit a tool_result block
 *
 * Thinking is part of the wire protocol but the current Rust forwarder
 * drops `thinking_delta` events (see `services/exec_agent/protocol/
 * corivo.rs`). Once that's wired up to a `thinking-delta` wire event,
 * extend `consume()` to grow Thinking blocks the same way Text blocks
 * grow.
 */
class BlockAccumulator {
  blocks: ContentBlock[] = [];
  citedFrameIds: string[] = [];
  finishReason: string | null = null;
  errorFromStream: string | null = null;
  cancelled = false;

  consume(event: ChatStreamEvent): void {
    switch (event.type) {
      case "text-delta": {
        const last = this.blocks[this.blocks.length - 1];
        if (last && last.type === "text") {
          this.blocks[this.blocks.length - 1] = {
            type: "text",
            text: last.text + event.text,
          };
        } else {
          this.blocks.push({ type: "text", text: event.text });
        }
        return;
      }
      case "cited-frames": {
        const seen = new Set(this.citedFrameIds);
        for (const id of event.frame_ids) seen.add(id);
        this.citedFrameIds = Array.from(seen);
        // Mirror as a content block too — preserves "the model cited
        // these in this position of the timeline" when re-rendering.
        this.blocks.push({
          type: "frame_citation",
          frame_ids: event.frame_ids,
        });
        return;
      }
      case "tool-call":
        this.blocks.push({
          type: "tool_use",
          id: event.id,
          name: event.name,
          input: event.args,
        });
        return;
      case "tool-result":
        this.blocks.push({
          type: "tool_result",
          tool_use_id: event.id,
          content: stringifyToolResult(event.result),
          is_error: false,
        });
        return;
      case "finish":
        switch (event.reason) {
          case "end-turn":
            this.finishReason = "EndTurn";
            break;
          case "truncated":
            this.finishReason = "Truncated";
            break;
          case "error":
            this.finishReason = "Error";
            break;
          case "cancelled":
            this.finishReason = "Cancelled";
            this.cancelled = true;
            break;
        }
        return;
      case "error":
        this.errorFromStream = event.message;
        return;
      case "status":
        // Status hints aren't part of the persisted timeline.
        return;
    }
  }

  toBlocks(): ContentBlock[] {
    return this.blocks;
  }
}

function stringifyToolResult(result: unknown): string {
  if (result === null || result === undefined) return "";
  if (typeof result === "string") return result;
  try {
    return JSON.stringify(result);
  } catch {
    return String(result);
  }
}

/**
 * Convert a persisted `ChatMessage` back into the `LiveChatMessage`
 * shape the renderer expects. Walks `content_blocks` to rebuild the
 * timeline of segments — text / thinking / tool clusters — preserving
 * the order the model actually emitted them in.
 */
function persistedToLive(persisted: ChatMessage): LiveChatMessage {
  const segments: AssistantSegment[] = [];
  const toolEvents: LiveToolEvent[] = [];
  let textRollup = "";

  for (const block of persisted.content_blocks) {
    switch (block.type) {
      case "text": {
        const last = segments[segments.length - 1];
        if (last && last.kind === "text") {
          last.text += block.text;
        } else {
          segments.push({ kind: "text", text: block.text });
        }
        textRollup += (textRollup ? "\n" : "") + block.text;
        break;
      }
      case "thinking": {
        // Thinking gets its own segment kind so the renderer can fold
        // it under a "思考过程" disclosure.
        const last = segments[segments.length - 1];
        if (last && last.kind === "thinking") {
          last.text += block.text;
        } else {
          segments.push({ kind: "thinking", text: block.text });
        }
        break;
      }
      case "tool_use": {
        const ev: LiveToolEvent = {
          id: block.id,
          name: block.name,
          args: (block.input ?? {}) as Record<string, unknown>,
          result: null,
        };
        toolEvents.push(ev);
        const last = segments[segments.length - 1];
        if (last && last.kind === "tools") {
          last.events.push(ev);
        } else {
          segments.push({ kind: "tools", events: [ev] });
        }
        break;
      }
      case "tool_result": {
        const target = toolEvents.find((t) => t.id === block.tool_use_id);
        if (target) {
          target.result = block.content;
        }
        // Patch existing cluster's matching event so the renderer sees
        // the result inline with the call.
        for (const seg of segments) {
          if (seg.kind !== "tools") continue;
          for (const e of seg.events) {
            if (e.id === block.tool_use_id) {
              e.result = block.content;
            }
          }
        }
        break;
      }
      case "focus_context":
      case "frame_citation":
        // User-side focus context and assistant-side frame citation
        // don't render as inline segments — the bubble surfaces frame
        // chips elsewhere via `cited_frame_ids`.
        break;
    }
  }

  return {
    id: persisted.id,
    role: persisted.role,
    content: textRollup || persisted.content_text,
    toolEvents,
    segments,
    citedFrameIds: persisted.cited_frame_ids ?? [],
    isStreaming: persisted.status === "streaming",
    error:
      persisted.status === "error"
        ? persisted.error_message ?? "stream finished with error"
        : undefined,
    modelUsed: persisted.model_used,
  };
}
