import { useEffect, useRef, useState } from "react"
import { useRouter, useRouterState } from "@tanstack/react-router"
import { Loader2, Plus, Search, Workflow, X } from "lucide-react"
import { Mascot } from "@/components/brand/mascot"
import { SidebarThreadList } from "@/components/layout/sidebar-thread-list"
import { StatusIndicator } from "@/components/layout/status-indicator"
import { UserCard } from "@/components/layout/user-card"
import { useWorkflowInFlightStore } from "@/stores/workflow-in-flight-store"
import { useTranslation } from "@/i18n"
import { useActiveThreadStore } from "@/stores/active-thread-store"

/**
 * App sidebar — IA after the bottom-rail redesign.
 *
 * Top:
 *   - Brand lockup
 *   - "+ 新会话"
 *   - Navigation row (我的工作流). The same slot becomes a transient
 *     search input on ⌘K — search is genuinely low-frequency for the
 *     thread list, so it doesn't deserve a permanent row.
 * Body:
 *   - Thread list (置顶 / 最近 / 归档)
 * Bottom:
 *   - Capture status pill (hero — amber-tinted when running)
 *   - User card (avatar + name + balance secondary line + dropdown)
 *   - Quick Ask ⌥⌥ microcopy (footer, no chrome)
 *
 * The bottom region intentionally has no dividers between rows; the
 * status pill carries the visual weight, the user card anchors the
 * card identity, and the footer hint is small enough to read as
 * background information.
 */
export function Sidebar() {
  const { t } = useTranslation()
  const router = useRouter()
  const pathname = useRouterState({ select: (s) => s.location.pathname })
  const openNew = useActiveThreadStore((s) => s.openNew)
  const hasDraft = useActiveThreadStore((s) => s.hasDraft)
  const searchQuery = useActiveThreadStore((s) => s.searchQuery)
  const setSearchQuery = useActiveThreadStore((s) => s.setSearchQuery)
  const [searchOpen, setSearchOpen] = useState(false)

  // "+ 新会话" and any thread-row click must bounce the user back to
  // `/ask` when the user is parked on a non-chat route. The active-
  // thread store is route-agnostic; without this jump the sidebar
  // interaction would silently update state with no visible surface
  // change.
  const ensureAskRoute = () => {
    if (pathname !== "/ask") {
      void router.navigate({ to: "/ask" })
    }
  }

  // Two rAFs: first lets `openNew` flush React state, second lands
  // after `MessageStream` mounts/swaps the textarea for the new
  // `inputKey` so the focus call actually finds it.
  const focusComposer = () => {
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        const el = document.querySelector<HTMLTextAreaElement>(
          "[data-composer-input]",
        )
        el?.focus()
      })
    })
  }

  // Global ⌘K — wherever focus is, drop the inline search input into
  // the slot just below "+ 新会话". Esc / ✕ collapses back. Search is
  // too low-frequency to deserve a permanent row, but ⌘K should always
  // work.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "k" && event.key !== "K") return
      if (!event.metaKey && !event.ctrlKey) return
      event.preventDefault()
      setSearchOpen(true)
    }
    document.addEventListener("keydown", onKey)
    return () => document.removeEventListener("keydown", onKey)
  }, [])

  // Global ⌘N / Ctrl+N — open a new chat draft, mirror of the
  // "+ 新会话" button. Captured in the renderer so the browser's
  // default ⌘N ("new window") never reaches WebKit. IME composition
  // guard keeps it from misfiring while picking a 拼音/かな/한글
  // candidate. Quick Ask owns its own ⌘N inside its window root —
  // when that overlay has focus, this listener never sees the event.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.isComposing || event.keyCode === 229) return
      if (event.key !== "n" && event.key !== "N") return
      if (!event.metaKey && !event.ctrlKey) return
      if (event.shiftKey || event.altKey) return
      event.preventDefault()
      if (pathname !== "/ask") {
        void router.navigate({ to: "/ask" })
      }
      openNew()
      focusComposer()
    }
    document.addEventListener("keydown", onKey)
    return () => document.removeEventListener("keydown", onKey)
  }, [openNew, pathname, router])

  const closeSearch = () => {
    setSearchQuery("")
    setSearchOpen(false)
  }

  return (
    <aside className="relative flex min-h-0 w-[232px] mt-7 shrink-0 flex-col gap-3 bg-transparent pb-3">
      <div className="flex flex-col gap-2 px-3 pt-1">
        <div className="flex items-center gap-2 px-1.5 pb-1">
          <Mascot size="sm" />
          <span className="font-display text-[14px] font-semibold leading-none tracking-[-0.01em] text-foreground">
            Corivo
          </span>
        </div>

        <button
          type="button"
          onClick={() => {
            ensureAskRoute()
            openNew()
            focusComposer()
          }}
          disabled={hasDraft}
          className="flex items-center gap-2 rounded-sm border border-[color-mix(in_oklab,var(--foreground)_13%,transparent)] bg-card px-2.5 py-1.5 text-[12.5px] font-medium tracking-[-0.005em] text-foreground transition-colors hover:border-[color-mix(in_oklab,var(--foreground)_22%,transparent)] disabled:opacity-50"
        >
          <Plus className="h-3.5 w-3.5 text-muted-foreground" />
          {t.ask.threadList.newThread}
        </button>

        {searchOpen ? (
          <SidebarSearchInput
            value={searchQuery}
            onChange={setSearchQuery}
            onClose={closeSearch}
          />
        ) : (
          <WorkflowsNav onSearch={() => setSearchOpen(true)} />
        )}
      </div>

      <SidebarThreadList />

      <div className="mt-auto flex flex-col gap-1.5 px-2 pt-2">
        <StatusIndicator />
        <UserCard />
        <QuickAskHint />
      </div>
    </aside>
  )
}

/**
 * Sidebar navigation row for the "我的工作流" route. Lives in the top
 * section, just below "+ 新会话", so route-level navigation reads as
 * primary IA rather than visually colliding with the bottom status
 * cluster (capture / account / hint).
 *
 * The right edge carries a small ⌘K affordance: clicking it (or the
 * global ⌘K hotkey) swaps this row out for the inline search input.
 */
function WorkflowsNav({ onSearch }: { onSearch: () => void }) {
  const { t } = useTranslation()
  const router = useRouter()
  const pathname = useRouterState({ select: (s) => s.location.pathname })
  const active = pathname.startsWith("/workflows")
  // Tiny in-flight indicator next to the nav label. Replaces the
  // dedicated "CORIVO 提议" sidebar section — that block confused
  // users ("what's this 提议 thing?") and duplicated information the
  // /workflows page already surfaces. A pulsing spinner inline here
  // keeps the "Corivo is busy" signal without the noise.
  const inFlightCount = useWorkflowInFlightStore((s) => s.entries.size)
  return (
    <div
      className={`group flex items-center gap-1 rounded-sm transition-colors ${
        active ? "bg-muted" : "hover:bg-muted/60"
      }`}
    >
      <button
        type="button"
        onClick={() => {
          if (!active) {
            void router.navigate({ to: "/workflows" })
          }
        }}
        aria-current={active ? "page" : undefined}
        className="flex min-w-0 flex-1 items-center gap-2 px-2.5 py-1.5 text-left"
      >
        <Workflow
          aria-hidden="true"
          className={`h-3.5 w-3.5 shrink-0 transition-colors ${
            active ? "text-foreground" : "text-muted-foreground"
          }`}
        />
        <span
          className={`min-w-0 flex-1 truncate text-[12.5px] font-medium tracking-[-0.005em] ${
            active ? "text-foreground" : "text-foreground/85"
          }`}
        >
          {t.sidebar.workflows}
        </span>
        {inFlightCount > 0 ? (
          <span
            className="flex shrink-0 items-center gap-0.5 text-[10.5px] text-muted-foreground"
            title={t.workflows.toast.running}
          >
            <Loader2 className="h-3 w-3 animate-spin" />
            {inFlightCount}
          </span>
        ) : null}
      </button>
      <button
        type="button"
        onClick={onSearch}
        aria-label={t.ask.threadList.searchPlaceholder}
        title={t.ask.threadList.searchPlaceholder}
        className="mr-1 flex shrink-0 items-center gap-1 rounded-[4px] px-1.5 py-1 text-muted-foreground transition-colors hover:bg-[color-mix(in_oklab,var(--foreground)_8%,transparent)] hover:text-foreground"
      >
        <Search className="h-3 w-3" />
        <span className="flex items-center gap-[2px]">
          <KbdMicro>⌘</KbdMicro>
          <KbdMicro>K</KbdMicro>
        </span>
      </button>
    </div>
  )
}

/**
 * Inline search input that takes over the workflows-nav slot while the
 * user is searching. Auto-focuses on mount; closes on Esc or the ✕
 * button. Blur is NOT a close trigger — the user routinely tabs into
 * the thread list to click a result.
 */
function SidebarSearchInput({
  value,
  onChange,
  onClose,
}: {
  value: string
  onChange: (next: string) => void
  onClose: () => void
}) {
  const { t } = useTranslation()
  const inputRef = useRef<HTMLInputElement | null>(null)

  useEffect(() => {
    inputRef.current?.focus()
    inputRef.current?.select()
  }, [])

  return (
    <div className="group flex items-center gap-2 rounded-sm border border-[color-mix(in_oklab,var(--foreground)_22%,transparent)] bg-card px-2.5 py-[7px] text-[12px]">
      <Search className="h-3 w-3 shrink-0 text-muted-foreground" />
      <input
        ref={inputRef}
        type="search"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            onClose()
          }
        }}
        placeholder={t.ask.threadList.searchPlaceholder}
        aria-label={t.ask.threadList.searchPlaceholder}
        className="flex-1 min-w-0 border-0 bg-transparent text-foreground placeholder:text-muted-foreground outline-none focus:outline-none focus-visible:outline-none focus-visible:!outline-0 focus-visible:!ring-0"
      />
      <button
        type="button"
        onClick={onClose}
        aria-label={t.common.cancel}
        className="flex h-4 w-4 shrink-0 items-center justify-center rounded text-muted-foreground hover:text-foreground"
      >
        <X className="h-3 w-3" />
      </button>
    </div>
  )
}

/**
 * Persistent Quick Ask discovery footer at the very bottom of the
 * sidebar. Reads as a full sentence — "双击 ⌥ 召唤 Corivo" — because
 * just rendering two ⌥ chips next to each other looked like an
 * inscrutable icon: users couldn't tell that the doubled chip meant
 * "tap this key twice." The literal "双击" / "Double-tap" word does
 * the work; the inline ⌥ chip identifies the key.
 *
 * Chrome-less microcopy: no border, no background, no padding
 * emphasis. The gesture is the highest-value behavior we teach, but
 * once the user has used it, this hint should fade into the
 * background — smallest interactive-looking element on the page.
 */
function QuickAskHint() {
  const { t } = useTranslation()
  return (
    <div
      className="mx-1 flex items-center gap-1 px-2 pt-0.5 text-[10.5px] text-muted-foreground/70"
      title={t.sidebar.quickAskHintTitle}
    >
      <span className="shrink-0">{t.sidebar.quickAskHintPrefix}</span>
      <KbdMicro>⌥</KbdMicro>
      <span className="truncate">{t.sidebar.quickAskHintSuffix}</span>
    </div>
  )
}

function KbdMicro({ children }: { children: React.ReactNode }) {
  return (
    <span className="inline-flex h-4 min-w-4 items-center justify-center rounded-[3px] bg-[color-mix(in_oklab,var(--foreground)_6%,transparent)] px-1 font-mono text-[9.5px] text-muted-foreground">
      {children}
    </span>
  )
}
