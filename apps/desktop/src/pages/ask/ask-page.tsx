import { useEffect, useMemo } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useRouter } from "@tanstack/react-router";

import {
  CHAT_QUERY_KEY,
  useChatStream,
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

/**
 * `/ask` page. The URL is the single source of truth for which thread
 * is on screen:
 *
 *   `/ask`                   → fresh draft (type-to-create)
 *   `/ask?threadId=<id>`     → viewing thread <id>
 *
 * Selecting / switching threads always goes through router navigation;
 * there is no parallel "active thread" field in any store. This is the
 * fix for the class of bugs where clicking "+ 新会话" landed the user
 * inside an existing thread — previously a deep-link effect re-synced
 * the URL's `?threadId` back into the store immediately after
 * `openNew()` had cleared it.
 */
export function AskPage() {
  const router = useRouter();
  const { threadId: urlThreadId } = AskRoute.useSearch();
  // Normalize "no thread" to `null` so downstream comparisons are
  // consistent (URL parser yields `undefined` when the param is absent).
  const threadId = urlThreadId ?? null;

  const migrateInputDraft = useActiveThreadStore((s) => s.migrateInputDraft);
  const setReadOnlyContext = useActiveThreadStore((s) => s.setReadOnlyContext);
  const markRead = useActiveThreadStore((s) => s.markRead);
  const pendingAutoSend = useActiveThreadStore((s) => s.pendingAutoSend);
  const setPendingAutoSend = useActiveThreadStore(
    (s) => s.setPendingAutoSend,
  );

  const qc = useQueryClient();

  // The workflow read-only banner is pinned to a specific thread id.
  // When the URL moves to anything else, drop the banner so the next
  // thread renders as a normal writable chat. The selection entry
  // points (sidebar workflow row, /workflows "查看历史") write the
  // context BEFORE navigating, so the matching threadId arrives in
  // the URL with the context already in place.
  useEffect(() => {
    const ctx = useActiveThreadStore.getState().readOnlyContext;
    if (ctx && ctx.threadId !== threadId) {
      setReadOnlyContext(null);
    }
  }, [threadId, setReadOnlyContext]);

  // Mark thread as read when the user opens it. Mirrors what the old
  // `setActive` reducer did inline — now driven off the URL instead.
  useEffect(() => {
    if (threadId) markRead(threadId);
  }, [threadId, markRead]);

  const streamOptions = useMemo(
    () => ({
      // Type-to-create: the URL stays at `/ask` (no threadId) until
      // the user actually sends. On send, we materialize a
      // `chat_thread` row, migrate the in-memory composer draft from
      // `DRAFT_THREAD_ID` onto the real id, and replace the URL so
      // the back button doesn't dump the user onto an empty draft.
      createThreadIfMissing: async () => {
        const created = await chatThreadCreate(PLACEHOLDER_THREAD_TITLE);
        return created.id;
      },
      onThreadCreated: (id: string) => {
        migrateInputDraft(DRAFT_THREAD_ID, id);
        void router.navigate({
          to: "/ask",
          search: { threadId: id },
          replace: true,
        });
        // Pop the new thread into the sidebar without waiting for the
        // post-turn invalidate, so the user sees their conversation
        // appear the moment they hit send.
        void qc.invalidateQueries({
          queryKey: [...CHAT_QUERY_KEY, "threads"],
        });
      },
    }),
    [qc, migrateInputDraft, router],
  );

  // Pass the URL threadId directly: `null` triggers the create-on-send
  // path inside the hook, a real id loads that thread's history.
  const stream = useChatStream(threadId, streamOptions);

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

  return (
    <MessageStream
      messages={stream.messages}
      isStreaming={stream.isStreaming}
      error={stream.error}
      onSend={(content) => stream.sendMessage(content)}
      onCancel={stream.cancel}
      inputKey={threadId ?? DRAFT_THREAD_ID}
    />
  );
}
