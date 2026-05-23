import { useMemo, useState } from "react";
import { useRouter, useRouterState } from "@tanstack/react-router";
import {
  Archive,
  ArchiveRestore,
  ChevronRight,
  MoreHorizontal,
  Pin,
  PinOff,
  Trash2,
} from "lucide-react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@repo/ui/components/dropdown-menu";
import { cn } from "@repo/ui/lib/utils";
import {
  useArchiveChatThread,
  useChatThreads,
  useDeleteChatThread,
  usePinChatThread,
} from "@/hooks/use-chat";
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
  const activeId = useActiveThreadStore((s) => s.activeId);
  const hasDraft = useActiveThreadStore((s) => s.hasDraft);
  const setActive = useActiveThreadStore((s) => s.setActive);
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

  // Apply search filter first, then bucket. Empty query → no filter.
  // Match against the title (case-insensitive). When the query is
  // active, the archived bucket auto-opens so search results in
  // archived threads are visible without an extra click.
  const trimmedQuery = searchQuery.trim().toLowerCase();
  const isSearching = trimmedQuery.length > 0;

  const { pinned, recent, archived } = useMemo(() => {
    const archived: ChatThread[] = [];
    const pinned: ChatThread[] = [];
    const recent: ChatThread[] = [];
    for (const thread of realThreads) {
      if (isSearching) {
        const haystack = (thread.title ?? "").toLowerCase();
        if (!haystack.includes(trimmedQuery)) continue;
      }
      if (thread.archived_at) {
        archived.push(thread);
      } else if (thread.pinned_at) {
        pinned.push(thread);
      } else {
        recent.push(thread);
      }
    }
    pinned.sort((a, b) =>
      (b.pinned_at ?? "").localeCompare(a.pinned_at ?? ""),
    );
    archived.sort((a, b) =>
      (b.archived_at ?? "").localeCompare(a.archived_at ?? ""),
    );
    return { pinned, recent, archived };
  }, [realThreads, isSearching, trimmedQuery]);

  const archivedExpanded = archivedOpen || isSearching;

  // Synthesise a draft entry at the head of the recent list when
  // the user has clicked "+ 新会话" but not yet sent. Stays in-memory
  // only — `chat_thread_create` runs on the first send via the
  // ask-page's `createThreadIfMissing` callback. Skip while searching
  // so the placeholder doesn't show up under a query that doesn't
  // match it.
  const recentWithDraft = useMemo<ChatThread[]>(() => {
    if (!hasDraft || isSearching) return recent;
    const now = new Date().toISOString();
    return [
      {
        id: DRAFT_THREAD_ID,
        title: PLACEHOLDER_THREAD_TITLE,
        bound_model_id: "",
        bound_api_shape: "anthropic",
        pinned_at: null,
        archived_at: null,
        created_at: now,
        updated_at: now,
      },
      ...recent,
    ];
  }, [hasDraft, recent, isSearching]);

  const displayActiveId = hasDraft ? DRAFT_THREAD_ID : activeId;

  const renderRow = (thread: ChatThread) => {
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
          {pinned.map(renderRow)}
        </Section>
      ) : null}

      {!isSearching || recent.length > 0 ? (
        <Section title={t.ask.threadList.recent}>
          {recentWithDraft.length === 0 ? (
            <p className="px-2 py-2 text-xs text-muted-foreground">
              {isSearching ? null : t.ask.threadList.empty}
            </p>
          ) : (
            recentWithDraft.map(renderRow)
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
            <div className="flex flex-col">{archived.map(renderRow)}</div>
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
      onContextMenu={(e) => {
        // Block the dev/Chromium default menu (Reload + Inspect
        // Element). For real threads, surface the same dropdown the
        // 3-dot trigger opens — drafts have no menu, so for them
        // we just suppress.
        e.preventDefault();
        if (!isDraft) setMenuOpen(true);
      }}
      className={cn(
        "group flex items-center gap-1 rounded-sm px-2 py-1.5 text-[12.5px] tracking-[-0.005em] transition-colors",
        active
          ? "bg-[var(--bg-deep,var(--muted))] font-medium text-foreground"
          : "text-muted-foreground hover:bg-muted hover:text-foreground",
      )}
    >
      <ActivityBadge kind={activityKind} />

      <button
        type="button"
        onClick={onSelect}
        className="min-w-0 flex-1 truncate text-left leading-tight"
      >
        {title}
      </button>

      {isDraft ? (
        // Draft: only delete (= dismiss) makes sense; no pin/archive.
        <button
          type="button"
          onClick={handleDelete}
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

function formatShortDate(iso: string): string {
  try {
    const d = new Date(iso);
    return d.toLocaleString();
  } catch {
    return iso;
  }
}
