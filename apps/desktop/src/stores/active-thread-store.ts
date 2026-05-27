import { create } from "zustand";

/**
 * Sentinel id used to key the composer draft (CO-63) when the user is
 * on `/ask` with no `?threadId=…` — i.e. the type-to-create slot.
 * Replaced by a real thread id on first send via `chat_thread_create`.
 * Never persisted.
 *
 * Routing note: the "is the user on a draft?" question is answered by
 * the URL alone (`pathname === "/ask" && !search.threadId`). This
 * constant only governs the per-row identity of the in-memory composer
 * text and the sidebar's draft placeholder row.
 */
export const DRAFT_THREAD_ID = "__draft__";

/**
 * Per-run context attached when the active thread is a workflow run
 * (kind='system', system_task='scheduled_workflow') instead of a
 * user-initiated chat. The chat viewer reads this to:
 *
 *   - Hide / disable the composer (workflow threads are read-only —
 *     the user can't push new turns into them).
 *   - Render a banner above the transcript with workflow name + the
 *     run's final status + duration, so the user knows the context
 *     ("this is the result of 每日回顾 from 11:31, succeeded in 24s")
 *     rather than seeing a bare conversation.
 *
 * `threadId` ties this context to a specific thread so it self-
 * invalidates when the user navigates to a different thread: consumers
 * check `ctx.threadId === currentThreadId` before rendering, and
 * AskPage clears the slot on any threadId mismatch. This replaces the
 * old "atomically write activeId + readOnlyContext" handshake — there
 * is no activeId in the store anymore, the URL is the truth.
 */
export interface ReadOnlyWorkflowRunContext {
  /** The chat_threads row this context applies to. Set by the workflow
   *  entry points (sidebar workflow row, /workflows "查看历史" button)
   *  alongside the navigation to `/ask?threadId=…`. */
  threadId: string;
  kind: "workflow_run";
  workflowName: string;
  /** slug of the workflow definition; used by the banner's "go back
   *  to manage" affordance to navigate to `/workflows`. */
  slug: string;
  runStatus: "success" | "failure";
  startedAt: string;
  finishedAt: string;
}

/**
 * Cross-cutting UI state for the chat surfaces. Deliberately does NOT
 * carry "which thread is active" — that lives in the URL
 * (`/ask?threadId=…`) and is read via TanStack Router. Routing the
 * active thread through both the URL and a store field was the root
 * of a class of races where clicking "+ 新会话" landed the user inside
 * the previously-selected thread (deep-link sync + auto-pick effects
 * fought the freshly-cleared store state).
 *
 * What's here:
 *   - `readOnlyContext` — pinned to one specific threadId so it self-
 *     invalidates when the user navigates away.
 *   - `unreadThreadIds` — CO-62 unread set, populated when an off-screen
 *     turn completes.
 *   - `searchQuery` — sidebar search input value.
 *   - `inputDrafts` — composer text per thread (CO-63), keyed by real
 *     thread id or `DRAFT_THREAD_ID` for the type-to-create slot.
 *   - `pendingAutoSend` — CO-68 one-shot prompt slot.
 */
interface ActiveThreadState {
  readOnlyContext: ReadOnlyWorkflowRunContext | null;
  /** Thread ids that have completed activity the user hasn't viewed
   *  yet (CO-62). Populated when `useChatStream.sendMessage` finishes
   *  a turn whose thread is not currently active, and cleared when
   *  the user opens the thread. In-memory only — cold boots start
   *  with zero unread, which matches the "completed work the user
   *  hasn't seen" semantics (work done in a previous session is
   *  already in the persisted message log, not a fresh agent reply
   *  to be flagged). */
  unreadThreadIds: string[];
  /** Sidebar search query. Empty string ↔ no filter (default). The
   *  Sidebar's `<input>` writes here, `SidebarThreadList` reads here
   *  to filter by thread title. ⌘K focuses the input — we don't
   *  open a separate command palette for v0. */
  searchQuery: string;
  /** Composer drafts per thread (CO-63). Key is the real thread id;
   *  the in-memory "+ 新会话" draft uses `DRAFT_THREAD_ID`. Without
   *  this, switching threads would leak whatever the user typed in
   *  thread A into thread B's composer, because `MessageStream`'s
   *  internal `useState` survives prop-driven thread switches inside
   *  the same `AskPage` mount. */
  inputDrafts: Record<string, string>;
  /** One-shot prompt that `AskPage` should fire as a user turn on its
   *  next render, without waiting for a manual Enter (CO-68). Set by
   *  flows like the Feishu CLI "安装" button that want to drop the user
   *  into a fresh `/ask` and have the agent start working immediately.
   *  The consumer clears it back to `null` synchronously when it picks
   *  up the value so the send fires exactly once. */
  pendingAutoSend: string | null;

  setReadOnlyContext: (ctx: ReadOnlyWorkflowRunContext | null) => void;
  setSearchQuery: (query: string) => void;
  setInputDraft: (key: string, value: string) => void;
  /** Move composer text from one key to another. Used when the draft
   *  thread materializes into a real `chat_threads` row on first send
   *  so a mid-flight reset (or a user switching away immediately) still
   *  finds their text under the thread it belongs to. */
  migrateInputDraft: (fromKey: string, toKey: string) => void;
  setPendingAutoSend: (value: string | null) => void;
  /** Flag a thread as having completed activity the user hasn't
   *  acknowledged. Called by `useChatStream.sendMessage` when a turn
   *  ends in a thread that is not currently active. */
  markUnread: (id: string) => void;
  /** Clear a thread from the unread set. Called when AskPage observes
   *  the URL threadId changing to `id` (the user is now looking at it). */
  markRead: (id: string) => void;
}

export const useActiveThreadStore = create<ActiveThreadState>((set) => ({
  readOnlyContext: null,
  searchQuery: "",
  inputDrafts: {},
  pendingAutoSend: null,
  unreadThreadIds: [],

  setReadOnlyContext: (ctx) => set({ readOnlyContext: ctx }),
  setSearchQuery: (query) => set({ searchQuery: query }),
  setInputDraft: (key, value) =>
    set((s) => {
      // Skip empty writes that don't change anything to keep the
      // reducer churn-free; this matters because `MessageStream`'s
      // submit path calls setInputDraft(key, "") on every send.
      if (!value && !s.inputDrafts[key]) return s;
      const nextDrafts = { ...s.inputDrafts };
      if (value) {
        nextDrafts[key] = value;
      } else {
        delete nextDrafts[key];
      }
      return { inputDrafts: nextDrafts };
    }),
  migrateInputDraft: (fromKey, toKey) =>
    set((s) => {
      const text = s.inputDrafts[fromKey];
      if (!text) return s;
      const nextDrafts = { ...s.inputDrafts };
      delete nextDrafts[fromKey];
      nextDrafts[toKey] = text;
      return { inputDrafts: nextDrafts };
    }),
  setPendingAutoSend: (value) => set({ pendingAutoSend: value }),
  markUnread: (id) =>
    set((s) => {
      if (s.unreadThreadIds.includes(id)) return s;
      return { unreadThreadIds: [...s.unreadThreadIds, id] };
    }),
  markRead: (id) =>
    set((s) => {
      if (!s.unreadThreadIds.includes(id)) return s;
      return {
        unreadThreadIds: s.unreadThreadIds.filter((other) => other !== id),
      };
    }),
}));
