import { create } from "zustand";

/**
 * Sentinel id surfaced in the sidebar after a "+ 新会话" click. Never
 * persisted; replaced by a real thread id on first send via
 * `chat_thread_create`. Mirrors the constant the ask-page used before
 * the sidebar refactor.
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
 */
export interface ReadOnlyWorkflowRunContext {
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
 * Active-thread state lives at the layout level so the Sidebar (which
 * renders the thread list) and the AskPage (which renders the active
 * conversation) read from the same source.
 *
 * Behavior:
 *   - `setActive(id)` selects a real thread (clears any prior
 *     read-only context — picking a regular chat thread is always a
 *     read/write surface).
 *   - `selectWorkflowRun(thread_id, ctx)` atomically sets activeId
 *     AND read-only context so the viewer switches into the
 *     workflow-run shape in one render.
 *   - `openNew()` flips `hasDraft` on and clears `activeId`. The DB
 *     row materializes only on first send.
 *   - `replaceDraftWith(id)` flips draft → real once the create-on-send
 *     path returns the new id.
 *   - `dismissDraft()` cancels a draft without sending (used by the
 *     "delete the draft entry" affordance in the sidebar).
 */
interface ActiveThreadState {
  activeId: string | null;
  hasDraft: boolean;
  /** Set when `activeId` points at a workflow run thread; `null`
   *  when it points at a normal user chat. The chat viewer reads
   *  this to swap into read-only mode. */
  readOnlyContext: ReadOnlyWorkflowRunContext | null;
  /** Whether the initial post-mount selection has run. The first
   *  render picks the most-recent thread; afterwards we honor
   *  whatever the user does. */
  initialized: boolean;
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

  setActive: (id: string | null) => void;
  /** Like `setActive`, but stamps the workflow-run context so the
   *  viewer switches into read-only mode atomically. Used by the
   *  unified sidebar when the user clicks a "⏰ 工作流" row, and by
   *  the workflows page's "查看历史" button. */
  selectWorkflowRun: (id: string, ctx: ReadOnlyWorkflowRunContext) => void;
  openNew: () => void;
  replaceDraftWith: (id: string) => void;
  dismissDraft: () => void;
  markInitialized: () => void;
  setSearchQuery: (query: string) => void;
  setInputDraft: (key: string, value: string) => void;
  setPendingAutoSend: (value: string | null) => void;
  /** Flag a thread as having completed activity the user hasn't
   *  acknowledged. Called by `useChatStream.sendMessage` when a turn
   *  ends in a thread that is not currently active. */
  markUnread: (id: string) => void;
}

export const useActiveThreadStore = create<ActiveThreadState>((set) => ({
  activeId: null,
  hasDraft: false,
  initialized: false,
  readOnlyContext: null,
  searchQuery: "",
  inputDrafts: {},
  pendingAutoSend: null,
  unreadThreadIds: [],

  setActive: (id) =>
    set((s) => ({
      activeId: id,
      hasDraft: false,
      // Picking a regular chat thread always exits read-only mode.
      // A subsequent `selectWorkflowRun` call re-enters it for a
      // workflow row.
      readOnlyContext: null,
      unreadThreadIds: id
        ? s.unreadThreadIds.filter((other) => other !== id)
        : s.unreadThreadIds,
    })),
  selectWorkflowRun: (id, ctx) =>
    set((s) => ({
      activeId: id,
      hasDraft: false,
      readOnlyContext: ctx,
      unreadThreadIds: s.unreadThreadIds.filter((other) => other !== id),
    })),
  openNew: () =>
    set({
      activeId: null,
      hasDraft: true,
      initialized: true,
      // Fresh draft is read/write by definition.
      readOnlyContext: null,
    }),
  replaceDraftWith: (id) =>
    set((s) => {
      // Migrate the pre-send draft text from DRAFT_THREAD_ID onto the
      // newly-minted real id so a stream that fails mid-flight (or a
      // user who switches away immediately after sending) still finds
      // their text under the thread it belongs to. We don't currently
      // hit this in practice because `submit()` clears the input before
      // awaiting send, but the migration keeps the contract clean for
      // future flows that don't pre-clear.
      const draftText = s.inputDrafts[DRAFT_THREAD_ID]
      const nextDrafts = { ...s.inputDrafts }
      delete nextDrafts[DRAFT_THREAD_ID]
      if (draftText) nextDrafts[id] = draftText
      return {
        activeId: id,
        hasDraft: false,
        initialized: true,
        inputDrafts: nextDrafts,
      }
    }),
  dismissDraft: () =>
    set((s) => {
      const nextDrafts = { ...s.inputDrafts }
      delete nextDrafts[DRAFT_THREAD_ID]
      return { hasDraft: false, inputDrafts: nextDrafts }
    }),
  markInitialized: () => set({ initialized: true }),
  setSearchQuery: (query) => set({ searchQuery: query }),
  setInputDraft: (key, value) =>
    set((s) => {
      // Skip empty writes that don't change anything to keep the
      // reducer churn-free; this matters because `MessageStream`'s
      // submit path calls setInputDraft(key, "") on every send.
      if (!value && !s.inputDrafts[key]) return s
      const nextDrafts = { ...s.inputDrafts }
      if (value) {
        nextDrafts[key] = value
      } else {
        delete nextDrafts[key]
      }
      return { inputDrafts: nextDrafts }
    }),
  setPendingAutoSend: (value) => set({ pendingAutoSend: value }),
  markUnread: (id) =>
    set((s) => {
      if (s.unreadThreadIds.includes(id)) return s
      return { unreadThreadIds: [...s.unreadThreadIds, id] }
    }),
}));
