import { create } from "zustand";

import type { LiveChatMessage } from "@/hooks/use-chat";

/**
 * In-flight stream state, keyed by thread id, lifted out of `useChatStream`
 * local state so it survives thread switches. Without this, switching to
 * another thread while a turn is streaming dropped the live buffer, and
 * switching back showed nothing until the assistant row was finalized in
 * the DB. With per-thread buckets the stream keeps writing here regardless
 * of which thread the UI is currently viewing.
 *
 * Lifecycle:
 *   - `startStream` opens a bucket on send (after the assistant placeholder
 *     row exists) so the live shape can render.
 *   - `updateLiveMessages` / `setError` / `setStreaming` are called from
 *     `channel.onmessage` and the surrounding try/catch in `sendMessage`.
 *   - `clearStream` is called after `chatAssistantMessageFinalize` +
 *     React Query invalidate — at that point the persisted row owns the
 *     canonical render, so the bucket must go away to avoid double-render.
 *
 * `BlockAccumulator` is mutable and not reactive, so it lives in a plain
 * module-scope Map in `use-chat.ts` next to where it's consumed — not here.
 */
interface ThreadStream {
  liveMessages: LiveChatMessage[];
  isStreaming: boolean;
  error: string | null;
}

const EMPTY_STREAM: ThreadStream = {
  liveMessages: [],
  isStreaming: false,
  error: null,
};

interface StreamingStore {
  byThreadId: Record<string, ThreadStream>;

  startStream: (threadId: string, initialMessage: LiveChatMessage) => void;
  updateLiveMessages: (
    threadId: string,
    updater: (prev: LiveChatMessage[]) => LiveChatMessage[],
  ) => void;
  setError: (threadId: string, error: string | null) => void;
  setStreaming: (threadId: string, isStreaming: boolean) => void;
  clearStream: (threadId: string) => void;
}

export const useStreamingStore = create<StreamingStore>((set) => ({
  byThreadId: {},

  startStream: (threadId, initialMessage) =>
    set((s) => ({
      byThreadId: {
        ...s.byThreadId,
        [threadId]: {
          liveMessages: [initialMessage],
          isStreaming: true,
          error: null,
        },
      },
    })),

  updateLiveMessages: (threadId, updater) =>
    set((s) => {
      const current = s.byThreadId[threadId];
      if (!current) return s;
      return {
        byThreadId: {
          ...s.byThreadId,
          [threadId]: {
            ...current,
            liveMessages: updater(current.liveMessages),
          },
        },
      };
    }),

  setError: (threadId, error) =>
    set((s) => {
      // setError must succeed even before `startStream` opened a bucket:
      // pre-stream failures (e.g. chat_user_message_create) need a place
      // to surface their error after `effectiveId` is known but the
      // assistant placeholder hasn't been created yet.
      const current = s.byThreadId[threadId] ?? EMPTY_STREAM;
      return {
        byThreadId: {
          ...s.byThreadId,
          [threadId]: { ...current, error },
        },
      };
    }),

  setStreaming: (threadId, isStreaming) =>
    set((s) => {
      const current = s.byThreadId[threadId];
      if (!current) return s;
      return {
        byThreadId: {
          ...s.byThreadId,
          [threadId]: { ...current, isStreaming },
        },
      };
    }),

  clearStream: (threadId) =>
    set((s) => {
      if (!(threadId in s.byThreadId)) return s;
      const next = { ...s.byThreadId };
      delete next[threadId];
      return { byThreadId: next };
    }),
}));

/** Stable empty bucket so the `useChatStream` selector returns the same
 *  reference when no stream is in flight for the current thread. */
export function selectThreadStream(
  state: StreamingStore,
  threadId: string | null,
): ThreadStream {
  if (!threadId) return EMPTY_STREAM;
  return state.byThreadId[threadId] ?? EMPTY_STREAM;
}
