import { useEffect, useMemo } from "react";
import { useQueryClient } from "@tanstack/react-query";

import {
  CHAT_QUERY_KEY,
  useChatStream,
  useChatThreads,
} from "@/hooks/use-chat";
import { chatThreadCreate } from "@/lib/tauri";
import { MessageStream } from "@/pages/ask/message-stream";
import { Route as AskRoute } from "@/routes/ask";
import {
  DRAFT_THREAD_ID,
  useActiveThreadStore,
} from "@/stores/active-thread-store";

// Must match PLACEHOLDER_THREAD_TITLE in
// apps/desktop/src-tauri/src/commands/chat.rs — the backend's auto-title
// path keys off this exact string when deciding whether to overwrite.
//
// The placeholder title is intentionally NOT translated: the backend
// uses it as a sentinel ("does this thread still have its placeholder?")
// to know whether to overwrite with the derived first-turn title. The
// localized rendering happens in `SidebarThreadList` via
// `t.ask.placeholderTitle`.
export const PLACEHOLDER_THREAD_TITLE = "新会话";

export function AskPage() {
  const { threadId: deepLinkThreadId } = AskRoute.useSearch();
  const activeId = useActiveThreadStore((s) => s.activeId);
  const hasDraft = useActiveThreadStore((s) => s.hasDraft);
  const initialized = useActiveThreadStore((s) => s.initialized);
  const setActive = useActiveThreadStore((s) => s.setActive);
  const replaceDraftWith = useActiveThreadStore((s) => s.replaceDraftWith);
  const markInitialized = useActiveThreadStore((s) => s.markInitialized);
  const pendingAutoSend = useActiveThreadStore((s) => s.pendingAutoSend);
  const setPendingAutoSend = useActiveThreadStore(
    (s) => s.setPendingAutoSend,
  );

  const threadsQuery = useChatThreads();
  const qc = useQueryClient();

  // Quick Ask deep-link: when Quick Ask emits `ask:open-thread`, AppBoot
  // navigates to `/ask?threadId=…` and the search param updates. Mirror
  // it into the active-thread store so the user can still switch threads
  // after.
  useEffect(() => {
    if (!deepLinkThreadId) return;
    if (deepLinkThreadId === activeId) return;
    setActive(deepLinkThreadId);
  }, [deepLinkThreadId, activeId, setActive]);

  // First-render: pick the most-recent thread if there is one, then
  // mark initialized so we never auto-select again — clicking 新会话
  // intentionally clears the selection and we don't want to fight that.
  useEffect(() => {
    if (initialized) return;
    if (activeId !== null) {
      markInitialized();
      return;
    }
    if (threadsQuery.isLoading) return;
    const threads = threadsQuery.data ?? [];
    if (threads.length > 0) {
      setActive(threads[0].id);
    } else {
      markInitialized();
    }
  }, [
    initialized,
    activeId,
    threadsQuery.isLoading,
    threadsQuery.data,
    setActive,
    markInitialized,
  ]);

  const streamOptions = useMemo(
    () => ({
      // Type-to-create: 新会话 only flips `hasDraft` on. The actual
      // chat_thread row materializes here, on the first send. We stamp
      // a placeholder title so the title-filtered list query surfaces
      // the new thread the moment it exists; chat_persist_turn replaces
      // this placeholder with a derived title once the assistant turn
      // lands.
      createThreadIfMissing: async () => {
        const created = await chatThreadCreate(PLACEHOLDER_THREAD_TITLE);
        return created.id;
      },
      onThreadCreated: (id: string) => {
        replaceDraftWith(id);
        // Pop the new thread into the sidebar without waiting for the
        // post-turn invalidate, so the user sees their conversation
        // appear the moment they hit send.
        void qc.invalidateQueries({
          queryKey: [...CHAT_QUERY_KEY, "threads"],
        });
      },
    }),
    [qc, replaceDraftWith],
  );

  // While a draft is active no real thread is selected — pass null to
  // the hook so it shows a blank conversation and the create-on-send
  // path runs.
  const stream = useChatStream(hasDraft ? null : activeId, streamOptions);

  // CO-68: one-shot auto-send. Flows like the Feishu CLI install button
  // stash a prompt in `pendingAutoSend` and navigate here; we fire it
  // as a user turn once and clear the slot synchronously so the effect
  // never repeats. The send goes through the same `createThreadIfMissing`
  // path as a manual submit, so a fresh `chat_thread` row is materialized
  // and the user's prompt + agent response both render normally.
  useEffect(() => {
    if (!pendingAutoSend) return;
    const content = pendingAutoSend;
    setPendingAutoSend(null);
    void stream.sendMessage(content);
  }, [pendingAutoSend, setPendingAutoSend, stream.sendMessage]);

  // Thread title used to render at the top of this column. Removed
  // 2026-05-09 — the sidebar already marks the active thread (amber
  // dot pin + bold), and a "X turn" count provided no real signal.
  // The conversation now starts at the very top of the column for
  // a calmer reading rhythm.
  return (
    <MessageStream
      messages={stream.messages}
      isStreaming={stream.isStreaming}
      error={stream.error}
      onSend={(content) => stream.sendMessage(content)}
      onCancel={stream.cancel}
      inputKey={hasDraft ? DRAFT_THREAD_ID : (activeId ?? DRAFT_THREAD_ID)}
    />
  );
}
