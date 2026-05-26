import { useMemo, useState } from "react";
import { useRouter, useRouterState } from "@tanstack/react-router";
import type { WorkflowRun, WorkflowView } from "@corivo/shared-types";
import {
  Archive,
  ArchiveRestore,
  Check,
  ChevronRight,
  MoreHorizontal,
  Pin,
  PinOff,
  Trash2,
  X,
} from "lucide-react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from "@repo/ui/components/dropdown-menu";
import { cn } from "@repo/ui/lib/utils";
import {
  useArchiveChatThread,
  useChatThreads,
  useDeleteChatThread,
  usePinChatThread,
} from "@/hooks/use-chat";
import { useWorkflowRuns, useWorkflowsList } from "@/hooks/use-workflows";
import { useTranslation } from "@/i18n";
import type { ChatThread } from "@/lib/types";
import { PLACEHOLDER_THREAD_TITLE } from "@/pages/ask/ask-page";
import {
  DRAFT_THREAD_ID,
  useActiveThreadStore,
} from "@/stores/active-thread-store";
import { useStreamingStore } from "@/stores/streaming-store";

/**
 * Thread list rendered inside the redesigned Sidebar (screens-v0 §02).
 * Three sections:
 *
 *   - 置顶  · pinned_at != null, sorted by pinned_at desc.
 *            Only rendered when at least one thread is pinned.
 *   - 最近  · neither pinned nor archived, sorted by updated_at desc.
 *            Includes the in-memory draft entry (placeholder) when
 *            "+ 新会话" has been clicked but no message sent yet.
 *   - 归档(N) · collapsible row at the bottom. Click to expand into
 *              the archived list. Only rendered when N > 0.
 *
 * Active row: tinted background + bold title; hover surfaces a 3-dot
 * action menu (pin/unpin · archive/unarchive · delete).
 */
export function SidebarThreadList() {
  const { t } = useTranslation();
  const router = useRouter();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const threadsQuery = useChatThreads();
  // Unified-history queries — see `UnifiedItem` notes below. Both load
  // in parallel with the chat thread list and run cheaply enough that
  // a sidebar render doesn't wait on them; if either is empty / still
  // loading, the sidebar just shows the chat side.
  const workflowRunsQuery = useWorkflowRuns(50);
  const workflowsQuery = useWorkflowsList();
  const activeId = useActiveThreadStore((s) => s.activeId);
  const hasDraft = useActiveThreadStore((s) => s.hasDraft);
  const setActive = useActiveThreadStore((s) => s.setActive);
  const selectWorkflowRun = useActiveThreadStore((s) => s.selectWorkflowRun);
  const dismissDraft = useActiveThreadStore((s) => s.dismissDraft);
  const searchQuery = useActiveThreadStore((s) => s.searchQuery);
  const unreadThreadIds = useActiveThreadStore((s) => s.unreadThreadIds);
  const streamingByThreadId = useStreamingStore((s) => s.byThreadId);
  const [archivedOpen, setArchivedOpen] = useState(false);

  // CO-62: each row's activity badge is computed from two sources —
  // the per-thread streaming bucket (running indicator) and the
  // unread-id set in the active-thread store (completed-but-unviewed
  // indicator). `running` surfaces even on the active row so a user
  // who navigates to /settings (or another thread) and comes back can
  // still tell at a glance whether the agent is mid-turn. `unread`
  // is suppressed on the active row because `setActive` already
  // clears the thread out of the unread set — you can't be on the
  // row AND have unviewed activity for it.
  const resolveActivityKind = (
    thread: ChatThread,
    isActive: boolean,
  ): ThreadActivityKind => {
    if (streamingByThreadId[thread.id]?.isStreaming) return "running";
    if (!isActive && unreadThreadIds.includes(thread.id)) return "unread";
    return "idle";
  };

  // Clicking a thread row updates the active-thread store but the
  // store is route-agnostic — when the user is parked on `/workflows`
  // (or any non-chat route) we have to send them back to `/ask` for
  // the selection to actually surface.
  const ensureAskRoute = () => {
    if (pathname !== "/ask") {
      void router.navigate({ to: "/ask" });
    }
  };

  const realThreads = threadsQuery.data ?? [];
  const workflowRuns = workflowRunsQuery.data ?? [];
  const workflows = workflowsQuery.data ?? [];

  // Apply search filter first, then bucket. Empty query → no filter.
  // Match against the title (case-insensitive). When the query is
  // active, the archived bucket auto-opens so search results in
  // archived threads are visible without an extra click.
  const trimmedQuery = searchQuery.trim().toLowerCase();
  const isSearching = trimmedQuery.length > 0;

  // Index workflows by slug so we can resolve each run's display name
  // in O(1) inside the merge loop. Empty Map when workflowsQuery
  // hasn't loaded yet — those rows fall back to the slug string.
  const workflowsBySlug = useMemo(() => {
    const map = new Map<string, WorkflowView>();
    for (const w of workflows) {
      map.set(w.definition.slug, w);
    }
    return map;
  }, [workflows]);

  // The sidebar surfaces three buckets:
  //
  //   pinned   — chat threads only. Workflows already have their own
  //              management surface (/workflows), so adding a pin
  //              affordance on a run row would duplicate state with
  //              no obvious meaning.
  //   recent   — mixed list of chat threads + workflow runs, sorted
  //              by their most-recent timestamp. The mental model is
  //              "everything Corivo has done recently in one feed".
  //   archived — chat threads only. Same reasoning as pinned.
  //
  // Workflow run rows are filtered against the search query by either
  // the workflow display name OR the slug — users may remember the
  // slug from the manage page even if the human-readable name doesn't
  // match.
  const { pinned, recent, archived } = useMemo(() => {
    const archived: UnifiedItem[] = [];
    const pinned: UnifiedItem[] = [];
    const recent: UnifiedItem[] = [];

    for (const thread of realThreads) {
      if (isSearching) {
        const haystack = (thread.title ?? "").toLowerCase();
        if (!haystack.includes(trimmedQuery)) continue;
      }
      const item: UnifiedItem = { kind: "chat", thread };
      if (thread.archived_at) {
        archived.push(item);
      } else if (thread.pinned_at) {
        pinned.push(item);
      } else {
        recent.push(item);
      }
    }

    for (const run of workflowRuns) {
      const def = workflowsBySlug.get(run.slug);
      const workflowName = def?.definition.name ?? run.slug;
      if (isSearching) {
        const haystack = `${workflowName} ${run.slug}`.toLowerCase();
        if (!haystack.includes(trimmedQuery)) continue;
      }
      recent.push({ kind: "workflow_run", run, workflowName });
    }

    pinned.sort((a, b) => timestamp(b).localeCompare(timestamp(a)));
    recent.sort((a, b) => timestamp(b).localeCompare(timestamp(a)));
    archived.sort((a, b) => timestamp(b).localeCompare(timestamp(a)));
    return { pinned, recent, archived };
  }, [realThreads, workflowRuns, workflowsBySlug, isSearching, trimmedQuery]);

  const archivedExpanded = archivedOpen || isSearching;

  // Synthesise a draft entry at the head of the recent list when
  // the user has clicked "+ 新会话" but not yet sent. Stays in-memory
  // only — `chat_thread_create` runs on the first send via the
  // ask-page's `createThreadIfMissing` callback. Skip while searching
  // so the placeholder doesn't show up under a query that doesn't
  // match it.
  const recentWithDraft = useMemo<UnifiedItem[]>(() => {
    if (!hasDraft || isSearching) return recent;
    const now = new Date().toISOString();
    const draftThread: ChatThread = {
      id: DRAFT_THREAD_ID,
      title: PLACEHOLDER_THREAD_TITLE,
      bound_model_id: "",
      bound_api_shape: "anthropic",
      pinned_at: null,
      archived_at: null,
      created_at: now,
      updated_at: now,
    };
    return [{ kind: "chat", thread: draftThread }, ...recent];
  }, [hasDraft, recent, isSearching]);

  const displayActiveId = hasDraft ? DRAFT_THREAD_ID : activeId;

  const renderItem = (item: UnifiedItem) => {
    if (item.kind === "chat") {
      const thread = item.thread;
      const isActive = thread.id === displayActiveId;
      const rawTitle = thread.title?.trim() ?? "";
      const displayTitle =
        rawTitle === PLACEHOLDER_THREAD_TITLE
          ? t.ask.placeholderTitle
          : rawTitle || formatShortDate(thread.updated_at);
      return (
        <ThreadRow
          key={thread.id}
          thread={thread}
          title={displayTitle}
          active={isActive}
          activityKind={resolveActivityKind(thread, isActive)}
          onSelect={() => {
            if (thread.id === DRAFT_THREAD_ID) return;
            ensureAskRoute();
            setActive(thread.id);
          }}
          onDismissDraft={dismissDraft}
          // After deleting the active thread, advance to the next one.
          onAfterDelete={(deletedId) => {
            if (deletedId === activeId) {
              const fallback = realThreads.find((other) => other.id !== deletedId);
              setActive(fallback?.id ?? null);
            }
          }}
        />
      );
    }
    // Workflow run row — read-only navigation target. Clicking
    // stamps `readOnlyContext` so the chat viewer renders the
    // transcript with a banner + no composer. Clicking the row
    // can't put a workflow run on the active id in a writable
    // way; the only way to "edit" is to go back to /workflows
    // and either edit the definition or hit Run Now again.
    const run = item.run;
    // run.thread_id is nullable when the runner failed before
    // creating its system chat thread. Skip the row in that case
    // — there's nothing to navigate to.
    if (!run.thread_id) return null;
    const isActive = run.thread_id === activeId;
    return (
      <WorkflowRunRow
        key={run.id}
        run={run}
        workflowName={item.workflowName}
        active={isActive}
        onSelect={() => {
          // run.thread_id is non-null here (checked above) — assert
          // for TS to narrow.
          const threadId = run.thread_id;
          if (!threadId) return;
          ensureAskRoute();
          selectWorkflowRun(threadId, {
            kind: "workflow_run",
            workflowName: item.workflowName,
            slug: run.slug,
            runStatus: run.status,
            startedAt: run.started_at,
            finishedAt: run.finished_at,
          });
        }}
      />
    );
  };

  const totalMatches = pinned.length + recent.length + archived.length;

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto px-2 pb-3">
      {isSearching && totalMatches === 0 ? (
        <p className="px-2 py-3 text-[12px] text-muted-foreground">
          {t.ask.threadList.searchEmpty(searchQuery)}
        </p>
      ) : null}

      {pinned.length > 0 ? (
        <Section title={t.ask.threadList.pinned}>
          {pinned.map(renderItem)}
        </Section>
      ) : null}

      {!isSearching || recent.length > 0 ? (
        <Section title={t.ask.threadList.recent}>
          {recentWithDraft.length === 0 ? (
            <p className="px-2 py-2 text-xs text-muted-foreground">
              {isSearching ? null : t.ask.threadList.empty}
            </p>
          ) : (
            recentWithDraft.map(renderItem)
          )}
        </Section>
      ) : null}

      {archived.length > 0 ? (
        <div className="mt-auto">
          <button
            type="button"
            onClick={() => setArchivedOpen((open) => !open)}
            className="flex w-full items-center gap-1.5 rounded-sm px-2 py-1.5 text-[11.5px] tracking-[-0.005em] text-muted-foreground transition-colors hover:text-foreground"
            aria-expanded={archivedExpanded}
          >
            <ChevronRight
              className={cn(
                "h-3 w-3 transition-transform",
                archivedExpanded && "rotate-90",
              )}
            />
            <span>
              {t.ask.threadList.archivedCount(archived.length)}
            </span>
          </button>
          {archivedExpanded ? (
            <div className="flex flex-col">{archived.map(renderItem)}</div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-0.5">
      <span className="px-2 pb-1 pt-2 font-mono text-[9.5px] font-medium uppercase tracking-[0.14em] text-muted-foreground">
        {title}
      </span>
      <div className="flex flex-col">{children}</div>
    </div>
  );
}

/**
 * What kind of activity badge to render on the row (CO-62).
 *   - `running`  — agent is currently streaming a turn here
 *   - `unread`   — a turn completed while the user was away
 *   - `error`    — last turn ended with an error (reserved; not wired
 *                  to a source yet — the streaming bucket clears as
 *                  soon as the message is finalized, so surfacing
 *                  this needs to thread the last-message status off
 *                  the persisted thread instead)
 *   - `idle`     — nothing to surface
 */
type ThreadActivityKind = "running" | "unread" | "error" | "idle";

function ThreadRow({
  thread,
  title,
  active,
  activityKind,
  onSelect,
  onDismissDraft,
  onAfterDelete,
}: {
  thread: ChatThread;
  title: string;
  active: boolean;
  activityKind: ThreadActivityKind;
  onSelect: () => void;
  onDismissDraft: () => void;
  onAfterDelete: (id: string) => void;
}) {
  const { t } = useTranslation();
  const pinMutation = usePinChatThread();
  const archiveMutation = useArchiveChatThread();
  const deleteMutation = useDeleteChatThread();
  // Controlled DropdownMenu so right-click on the row can open it
  // alongside the 3-dot trigger. Without this, right-click would
  // either fall through to Chromium's default context menu (Reload /
  // Inspect Element on dev builds) or do nothing.
  const [menuOpen, setMenuOpen] = useState(false);

  const isDraft = thread.id === DRAFT_THREAD_ID;
  const isPinned = !!thread.pinned_at;
  const isArchived = !!thread.archived_at;

  const handleDelete = () => {
    if (isDraft) {
      onDismissDraft();
      return;
    }
    deleteMutation.mutate(thread.id, {
      onSuccess: () => onAfterDelete(thread.id),
    });
  };

  return (
    <div
      role="button"
      tabIndex={0}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
      onContextMenu={(e) => {
        // Block the dev/Chromium default menu (Reload + Inspect
        // Element). For real threads, surface the same dropdown the
        // 3-dot trigger opens — drafts have no menu, so for them
        // we just suppress.
        e.preventDefault();
        if (!isDraft) setMenuOpen(true);
      }}
      className={cn(
        "group flex cursor-pointer items-center gap-1 rounded-sm px-2 py-1.5 text-[12.5px] tracking-[-0.005em] transition-colors",
        active
          ? "bg-[var(--bg-deep,var(--muted))] font-medium text-foreground"
          : "text-muted-foreground hover:bg-muted hover:text-foreground",
      )}
    >
      <ActivityBadge kind={activityKind} />

      <span className="min-w-0 flex-1 truncate text-left leading-tight">
        {title}
      </span>

      {isDraft ? (
        // Draft: only delete (= dismiss) makes sense; no pin/archive.
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            handleDelete();
          }}
          aria-label={t.ask.threadList.action.delete}
          className="invisible flex h-5 w-5 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-[var(--bg-deep,var(--muted))] hover:text-foreground group-hover:visible"
        >
          <Trash2 className="h-3 w-3" />
        </button>
      ) : (
        <DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              aria-label={t.ask.threadList.action.menu}
              onClick={(e) => e.stopPropagation()}
              className="invisible flex h-5 w-5 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-[var(--bg-deep,var(--muted))] hover:text-foreground data-[state=open]:visible group-hover:visible"
            >
              <MoreHorizontal className="h-3.5 w-3.5" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            side="right"
            align="start"
            sideOffset={4}
            className="w-44"
            onKeyDown={(e) => {
              // Single-key shortcuts (P / A / D) while the menu has
              // focus. Ignore modifier combos so they don't collide
              // with Cmd+A "select all" etc., and ignore repeats so
              // holding a key doesn't double-fire the mutation.
              if (e.metaKey || e.ctrlKey || e.altKey || e.repeat) return;
              const key = e.key.toLowerCase();
              if (key === "p") {
                e.preventDefault();
                pinMutation.mutate({ id: thread.id, pinned: !isPinned });
                setMenuOpen(false);
              } else if (key === "a") {
                e.preventDefault();
                archiveMutation.mutate({
                  id: thread.id,
                  archived: !isArchived,
                });
                setMenuOpen(false);
              } else if (key === "d") {
                e.preventDefault();
                handleDelete();
                setMenuOpen(false);
              }
            }}
          >
            {isArchived ? (
              <DropdownMenuItem
                onClick={() =>
                  archiveMutation.mutate({
                    id: thread.id,
                    archived: false,
                  })
                }
                className="gap-2"
              >
                <ArchiveRestore className="h-3.5 w-3.5" />
                {t.ask.threadList.action.unarchive}
                <DropdownMenuShortcut>A</DropdownMenuShortcut>
              </DropdownMenuItem>
            ) : (
              <>
                <DropdownMenuItem
                  onClick={() =>
                    pinMutation.mutate({
                      id: thread.id,
                      pinned: !isPinned,
                    })
                  }
                  className="gap-2"
                >
                  {isPinned ? (
                    <PinOff className="h-3.5 w-3.5" />
                  ) : (
                    <Pin className="h-3.5 w-3.5" />
                  )}
                  {isPinned
                    ? t.ask.threadList.action.unpin
                    : t.ask.threadList.action.pin}
                  <DropdownMenuShortcut>P</DropdownMenuShortcut>
                </DropdownMenuItem>
                <DropdownMenuItem
                  onClick={() =>
                    archiveMutation.mutate({
                      id: thread.id,
                      archived: true,
                    })
                  }
                  className="gap-2"
                >
                  <Archive className="h-3.5 w-3.5" />
                  {t.ask.threadList.action.archive}
                  <DropdownMenuShortcut>A</DropdownMenuShortcut>
                </DropdownMenuItem>
              </>
            )}
            <DropdownMenuSeparator />
            <DropdownMenuItem
              onClick={handleDelete}
              className="gap-2 text-destructive focus:text-destructive"
            >
              <Trash2 className="h-3.5 w-3.5" />
              {t.ask.threadList.action.delete}
              <DropdownMenuShortcut>D</DropdownMenuShortcut>
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      )}
    </div>
  );
}

/**
 * Leading activity indicator (CO-62). Three states share one visual
 * container (1.5px dot, fixed slot left of the title) so they read
 * as one language with three colors:
 *
 *   running → amber + `corivo-status-pulse` (matches the capture
 *             status dot at the sidebar foot — "something is alive")
 *   unread  → muted ink-blue, solid (mac-mail "unread" idiom; cool
 *             color separates it from the warm running state)
 *   error   → destructive red, solid (reserved; see ThreadActivityKind)
 *
 * `idle` still renders a transparent dot of the same size so every
 * row's title baseline starts at the same x — otherwise the list
 * would visually jitter as threads flip between idle/active states.
 */
function ActivityBadge({ kind }: { kind: ThreadActivityKind }) {
  const { t } = useTranslation();
  if (kind === "idle") {
    return (
      <span aria-hidden className="inline-block h-1.5 w-1.5 shrink-0" />
    );
  }
  const colorClass =
    kind === "running"
      ? "bg-[var(--corivo-amber)]"
      : kind === "unread"
        ? "bg-[var(--corivo-unread)]"
        : "bg-destructive";
  const label =
    kind === "running"
      ? t.ask.threadList.activity.running
      : kind === "unread"
        ? t.ask.threadList.activity.unread
        : t.ask.threadList.activity.error;
  return (
    <span
      role="img"
      aria-label={label}
      title={label}
      className={cn(
        "inline-block h-1.5 w-1.5 shrink-0 rounded-full",
        colorClass,
      )}
      style={
        kind === "running"
          ? { animation: "corivo-status-pulse 2.4s ease-in-out infinite" }
          : undefined
      }
    />
  );
}

/**
 * Tagged union of "things that show up in the sidebar history list".
 * The chat side carries a real ChatThread row from the DB; the
 * workflow side carries a WorkflowRun + the workflow's resolved
 * display name (looked up by slug from `useWorkflowsList`). Both end
 * up sorted by a single timestamp scalar (`timestamp()` below).
 */
type UnifiedItem =
  | { kind: "chat"; thread: ChatThread }
  | {
      kind: "workflow_run";
      run: WorkflowRun;
      /** Human-friendly name from WORKFLOW.md frontmatter; falls
       *  back to the slug when the workflows list query hasn't
       *  loaded yet (cold sidebar render). */
      workflowName: string;
    };

/** Sort key for `UnifiedItem`. Chat threads use `updated_at` (last
 *  activity); workflow runs use `started_at` (when the run kicked
 *  off — `finished_at` would also work since runs are short, but
 *  started_at matches the user's mental model of "when did this
 *  show up"). */
function timestamp(item: UnifiedItem): string {
  return item.kind === "chat" ? item.thread.updated_at : item.run.started_at;
}

/**
 * Sidebar row for a workflow run. Visually echoes ThreadRow but
 * carries the always-present clock prefix + a tail status pill
 * (✓/✗) so the user can spot success vs failure without opening
 * the transcript. Right-side action menu is intentionally absent:
 * pin / archive / delete don't have a clear meaning on a run
 * (the underlying definition lives on disk, the run row lives in
 * SQLite, the relationship is "many runs per definition"); if the
 * user wants to clear noise, they delete the workflow definition
 * from /workflows and the FK cascade drops the runs with it.
 */
function WorkflowRunRow({
  run,
  workflowName,
  active,
  onSelect,
}: {
  run: WorkflowRun;
  workflowName: string;
  active: boolean;
  onSelect: () => void;
}) {
  const { t } = useTranslation();
  const wf = t.ask.threadList.workflow;
  const isFailure = run.status === "failure";
  // Title: "⏰ 每日回顾". The clock glyph lives in the i18n bundle
  // so a future swap to a lucide icon is a one-line change.
  const title = `${wf.prefixIcon} ${workflowName}`;
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
      className={cn(
        "group flex cursor-pointer items-center gap-1 rounded-sm px-2 py-1.5 text-[12.5px] tracking-[-0.005em] transition-colors",
        active
          ? "bg-[var(--bg-deep,var(--muted))] font-medium text-foreground"
          : "text-muted-foreground hover:bg-muted hover:text-foreground",
      )}
    >
      {/* Empty 1.5×1.5 slot keeps the title baseline aligned with
          ThreadRow's ActivityBadge column. Cleaner than introducing a
          conditional class on the parent. */}
      <span aria-hidden className="inline-block h-1.5 w-1.5 shrink-0" />

      <span className="min-w-0 flex-1 truncate text-left leading-tight">
        {title}
      </span>

      <span
        title={isFailure ? wf.statusFailure : wf.statusSuccess}
        aria-label={isFailure ? wf.statusFailure : wf.statusSuccess}
        className={cn(
          "flex h-4 w-4 shrink-0 items-center justify-center rounded-full",
          isFailure
            ? "bg-destructive/10 text-destructive"
            : "bg-emerald-100 text-emerald-700 dark:bg-emerald-950/40 dark:text-emerald-300",
        )}
      >
        {isFailure ? <X className="h-2.5 w-2.5" /> : <Check className="h-2.5 w-2.5" />}
      </span>
    </div>
  );
}

function formatShortDate(iso: string): string {
  try {
    const d = new Date(iso);
    return d.toLocaleString();
  } catch {
    return iso;
  }
}
