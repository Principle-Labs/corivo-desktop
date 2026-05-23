import { LogicalSize } from "@tauri-apps/api/dpi";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import {
  KeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import { MessageBubble } from "@/components/chat/message-bubble";
import {
  useChatStream,
  useChatStreamMirror,
  useChatThreads,
  useChatThreadsSync,
} from "@/hooks/use-chat";
import { useConfigSync } from "@/hooks/use-config";
import { usePermissionListener } from "@/hooks/use-permission-listener";
import { useTranslation, type LocaleDict } from "@/i18n";
import {
  chatThreadCreate,
  quickAskCaptureFocus,
  quickAskHide,
  quickAskLogDisplay,
  quickAskOpenInApp,
  quickAskSetHeight,
} from "@/lib/tauri";
import type {
  ChatThread,
  FocusContext,
  QuickAskOpenedPayload,
  QuickAskSkeleton,
} from "@/lib/types";
import { FocusCard } from "@/overlay-quick-ask/FocusCard";
import { QuickAskPermissionCard } from "@/overlay-quick-ask/QuickAskPermissionCard";

const OPENED_EVENT = "quick-ask:opened";
const FOCUS_READY_EVENT = "quick-ask:focus-ready";
const ERROR_EVENT = "quick-ask:error";
const FRAME_INGESTED_EVENT = "capture:frame-ingested";
// Fired by the NSWorkspace observer the instant a new app becomes
// frontmost — *before* the capture pipeline finishes ingesting the
// frame. We use it to flip the FocusCard into a skeleton state right
// away ("正在切换到 VSCode…"); the full FocusContext arrives after
// AX/OCR via `capture:frame-ingested`. The backend already filters
// out the Corivo process itself on both events (NSWorkspace observer
// uses `pid == our_pid`; LocalConsumer uses `app.config().identifier`),
// so this side does not need its own bundle-id guard.
const FOCUS_ACTIVATED_EVENT = "capture:focus-activated";

// Manual-resize bounds for the bottom-right drag handle. Width tracks
// tauri.conf.json's min/maxWidth so AppKit won't override. Max height
// matches commands/quick_ask.rs's QUICK_ASK_MAX_HEIGHT so manual and
// auto-fit share one ceiling. Min height intentionally diverges from
// the Rust QUICK_ASK_MIN_HEIGHT (100, a safety floor that auto-fit
// never reaches in practice — natural content sits at ~176): the
// manual handle floor lives above natural content so the composer
// doesn't overflow when the user drags inward.
const QUICK_ASK_MIN_WIDTH = 480;
const QUICK_ASK_MAX_WIDTH = 1200;
const QUICK_ASK_MIN_HEIGHT = 220;
const QUICK_ASK_MAX_HEIGHT = 800;

interface FocusActivatedPayload {
  app_bundle_id: string | null;
  app_name: string | null;
  pid: number | null;
}

export function QuickAskWindow() {
  const { t } = useTranslation();
  // Subscribe this overlay's QueryClient to chat:threads-changed
  // broadcasts so a write from the main window invalidates our local
  // thread cache (and vice-versa). Symmetrical to the AppBoot mount.
  useChatThreadsSync();
  // Mirror any in-flight stream owned by the main /ask window so a
  // turn fired there renders live in this overlay too.
  useChatStreamMirror();
  // And to config:changed so a language flip in the main window
  // immediately re-renders this overlay in the new dictionary.
  useConfigSync();
  // Thread id is null until the user actually sends. `useChatStream`'s
  // `createThreadIfMissing` mints a `chat_threads` row on the first send
  // and `onThreadCreated` syncs it back here — same type-to-create flow
  // `/ask` uses ([ask-page.tsx](../pages/ask/ask-page.tsx)). Means a
  // hotkey press the user closes without sending leaves no row behind.
  const [threadId, setThreadId] = useState<string | null>(null);
  // `skeleton` lands first (right after window.show()); the focus state
  // splits in two for CO-3:
  //   - `attachedFocus` is the **locked** frame that will be sent with
  //     the next message. Initialized from the summon focus and kept
  //     stable while the panel is open — no auto-drift to whichever
  //     app the user happens to glance at. The user explicitly swaps
  //     it via the "+ 切换到 [App]" chip (see `liveFocus` below).
  //   - `liveFocus` tracks the *current* foreground app via
  //     `capture:focus-activated` / `capture:frame-ingested`. Drives
  //     the swap chip when it diverges from `attachedFocus`.
  // Send is gated on `attachedFocus` because the recall orchestrator
  // needs `focus.frame_id`.
  const [skeleton, setSkeleton] = useState<QuickAskSkeleton | null>(null);
  const [attachedFocus, setAttachedFocus] = useState<FocusContext | null>(null);
  const [liveFocus, setLiveFocus] = useState<FocusContext | null>(null);
  const [loadingFocus, setLoadingFocus] = useState(false);
  const [draft, setDraft] = useState("");
  const [bootstrapError, setBootstrapError] = useState<string | null>(null);
  // Toggles the session-picker panel between the FocusCard and the
  // input bar. The panel reuses the persisted `chat_threads` list, so
  // every Quick Ask / `/ask` thread is reachable here.
  const [pickerOpen, setPickerOpen] = useState(false);
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const transcriptRef = useRef<HTMLDivElement | null>(null);
  const threadsQuery = useChatThreads();
  // Permission requests from corivo-agent. The Rust bridge routes
  // `exec-agent:permission-request` events to this window's label
  // ("quick-ask") whenever the turn was started from QA, so the prompt
  // surfaces here instead of in the main app.
  const permission = usePermissionListener();

  // Same hook the `/ask` page uses — Quick Ask just adds the focus
  // preamble per turn and passes through the same stream events. Tool
  // calls + thinking-with-newlines + persistence all come for free.
  const stream = useChatStream(threadId, {
    createThreadIfMissing: async () => {
      const created = await chatThreadCreate(null);
      return created.id;
    },
    onThreadCreated: setThreadId,
  });

  // `stream` is a fresh object literal every render, so we deliberately
  // do NOT use it as an effect dep — that would tear down + re-register
  // the Tauri listeners below on every render, opening a race window
  // where events fired during the gap are lost. The listener effect
  // captures `stream.reset` via a ref so it always sees the latest
  // closure without re-running.
  const streamRef = useRef(stream);
  streamRef.current = stream;

  // Stick the transcript to the bottom whenever messages change — both
  // when the user sends (so their own bubble appears in view) and as
  // streaming tokens arrive. Mirrors the `/ask` page pattern in
  // [message-stream.tsx](../pages/ask/message-stream.tsx).
  useEffect(() => {
    const el = transcriptRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [stream.messages]);

  // Two-phase open from the hotkey (lib.rs::on_quick_ask_hotkey):
  //   1. `quick-ask:opened` — fires immediately after window.show().
  //      Carries either a skeleton (AX walk in flight) or a full focus
  //      payload (excluded apps short-circuit).
  //   2. `quick-ask:focus-ready` — fires when phase B finishes and
  //      delivers the full FocusContext with frame_id + primary_text.
  //
  // Standard "cancelled flag + Promise.all" pattern for Tauri
  // listeners. React 18 StrictMode mounts dev components twice
  // (mount → cleanup → mount), and `listen()` is an async
  // round-trip to Rust — without this guard the cleanup of the first
  // mount races against the setup of the second, leaving listener
  // state in the backend inconsistent and events get dropped.
  useEffect(() => {
    let cancelled = false;
    let unlisteners: UnlistenFn[] = [];

    const onOpened = (event: { payload: QuickAskOpenedPayload }) => {
      const payload = event.payload;
      // Each hotkey invoke is a fresh conversation: drop the previous
      // thread id so `useChatStream` mints a new row on the next send.
      setThreadId(null);
      setBootstrapError(null);
      setDraft("");
      setPickerOpen(false);
      setManualResized(false);
      streamRef.current.reset();
      if (payload.kind === "ready") {
        setSkeleton(null);
        // Locked: `attachedFocus` becomes the summon frame and stays
        // put until the user explicitly swaps it. `liveFocus` mirrors
        // it initially so the swap chip stays hidden until divergence.
        setAttachedFocus(payload.focus);
        setLiveFocus(payload.focus);
        setLoadingFocus(false);
      } else {
        setSkeleton(payload.skeleton);
        setAttachedFocus(null);
        setLiveFocus(null);
        setLoadingFocus(true);
      }
      // Defer focus to next tick so the textarea is mounted.
      requestAnimationFrame(() => inputRef.current?.focus());
    };

    const onReady = (event: { payload: FocusContext }) => {
      // Phase B for the *summon* — adopt the full focus as the locked
      // anchor for the upcoming first message.
      setAttachedFocus(event.payload);
      setLiveFocus(event.payload);
      setSkeleton(null);
      setLoadingFocus(false);
    };

    const onError = (event: { payload: string }) => {
      setLoadingFocus(false);
      setBootstrapError(event.payload);
    };

    // Phase A: NSWorkspace observed the user activated a new app.
    // Update `liveFocus` ONLY — `attachedFocus` stays locked at the
    // summon frame so the FocusCard doesn't drift. The swap chip uses
    // `liveFocus` to advertise "+ 切换到 [App]" the user can click to
    // re-anchor for the next message (CO-3 #3). Backend already
    // filtered out the Corivo process itself.
    const onActivated = (event: { payload: FocusActivatedPayload }) => {
      const next = event.payload;
      setLiveFocus(null);
      setSkeleton({
        captured_at: new Date().toISOString(),
        app_bundle_id: next.app_bundle_id,
        app_name: next.app_name,
        window_title: null,
        url: null,
      });
    };

    // Phase B: capture pipeline finished ingesting the frame for the
    // new foreground app. Update `liveFocus`; leave `attachedFocus`
    // alone unless the user clicks the swap chip.
    const onFrame = (event: { payload: FocusContext }) => {
      setLiveFocus(event.payload);
      setSkeleton(null);
      setLoadingFocus(false);
    };

    Promise.all([
      listen<QuickAskOpenedPayload>(OPENED_EVENT, onOpened),
      listen<FocusContext>(FOCUS_READY_EVENT, onReady),
      listen<string>(ERROR_EVENT, onError),
      listen<FocusActivatedPayload>(FOCUS_ACTIVATED_EVENT, onActivated),
      listen<FocusContext>(FRAME_INGESTED_EVENT, onFrame),
    ])
      .then((fns) => {
        if (cancelled) {
          // Cleanup ran before listen() resolved — tear down right now.
          fns.forEach((u) => u());
        } else {
          unlisteners = fns;
        }
      })
      .catch((e) => {
        console.warn("[quick-ask] listen registration failed", e);
      });

    return () => {
      cancelled = true;
      unlisteners.forEach((u) => u());
    };
  }, []);

  // First-load fallback: if the window was opened directly (not via the
  // hotkey), kick off a capture ourselves so the user isn't staring at
  // a blank panel. This goes through the synchronous IPC which still
  // returns a full FocusContext in one shot. No thread is minted here —
  // it'll be created on the first send.
  useEffect(() => {
    if (attachedFocus !== null) return;
    setLoadingFocus(true);
    quickAskCaptureFocus()
      .then((nextFocus) => {
        setSkeleton(null);
        setAttachedFocus(nextFocus);
        setLiveFocus(nextFocus);
        setBootstrapError(null);
      })
      .catch((e) => {
        setBootstrapError(String(e));
      })
      .finally(() => setLoadingFocus(false));
    // Run only on mount; subsequent opens go through the listen hook.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Accepts an optional `explicit` override so the suggestion chips
  // can fire a prompt without round-tripping through the textarea
  // (which would race React state updates against the click).
  const submit = useCallback(
    (explicit?: string) => {
      const text = (explicit ?? draft).trim();
      if (!text || !attachedFocus) return;
      setDraft("");
      void stream.sendMessage(
        text,
        {
          frameId: attachedFocus.frame_id,
          summary: focusSummary(attachedFocus),
          primaryText: attachedFocus.primary_text,
          selection: attachedFocus.selection,
        },
        shortFocusLabel(attachedFocus, t),
      );

    },
    [draft, attachedFocus, stream, t],
  );

  // Re-anchor the next message to the user's *current* foreground app
  // (CO-3 #3). Triggered by the "+ 切换到 [App]" chip when `liveFocus`
  // diverges from `attachedFocus`.
  const adoptLiveFocus = useCallback(() => {
    if (!liveFocus) return;
    setAttachedFocus(liveFocus);
  }, [liveFocus]);

  // While a permission card is up, ESC must not close the window — that
  // would orphan the agent on a never-resolved request and the user
  // would lose visibility into what they were being asked. The card has
  // its own explicit Deny button.
  const hasPendingPermission = permission.current !== null;
  const pendingPermissionRef = useRef(hasPendingPermission);
  pendingPermissionRef.current = hasPendingPermission;

  // ESC priority: dismiss the history overlay (when open) before closing
  // the window, so users can peek at history and back out without losing
  // the active conversation. Mirror the live value into a ref for the
  // window-level fallback listener, which runs outside React state.
  const pickerOpenRef = useRef(pickerOpen);
  pickerOpenRef.current = pickerOpen;

  // ESC or Cmd/Ctrl+W closes the window (unless a permission card is up);
  // Enter sends, Shift+Enter inserts a newline. When the history overlay
  // is open, ESC dismisses it first instead of closing the window.
  const onKeyDown = useCallback(
    (event: KeyboardEvent<HTMLTextAreaElement>) => {
      // While an IME is composing (拼音/かな/한글 picking a candidate),
      // Enter means "confirm the candidate", not "send the message".
      // `keyCode === 229` is the legacy signal for the Enter that closes
      // composition — some WebKit builds flip `isComposing` back to false
      // on that final keydown, so we need both. Matches /ask composer
      // ([message-stream.tsx](../pages/ask/message-stream.tsx)).
      if (event.nativeEvent.isComposing || event.keyCode === 229) return;
      const isCloseShortcut =
        event.key === "Escape" ||
        (event.key === "w" && (event.metaKey || event.ctrlKey));
      if (isCloseShortcut) {
        event.preventDefault();
        if (hasPendingPermission) return;
        if (event.key === "Escape" && pickerOpen) {
          setPickerOpen(false);
          return;
        }
        void quickAskHide();
        return;
      }
      if (event.key === "Enter" && !event.shiftKey) {
        event.preventDefault();
        submit();
      }
    },
    [submit, hasPendingPermission, pickerOpen],
  );

  // Window-level close-shortcut fallback (in case the textarea isn't focused).
  useEffect(() => {
    const handler = (event: globalThis.KeyboardEvent) => {
      const isCloseShortcut =
        event.key === "Escape" ||
        (event.key === "w" && (event.metaKey || event.ctrlKey));
      if (isCloseShortcut) {
        event.preventDefault();
        if (pendingPermissionRef.current) return;
        if (event.key === "Escape" && pickerOpenRef.current) {
          setPickerOpen(false);
          return;
        }
        void quickAskHide();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  // Window height tracks rendered content via ResizeObserver until
  // the user manually resizes (drags the bottom-right corner). After
  // that we stop pushing back so the user-chosen size sticks. We also
  // re-arm auto-height when the panel is summoned again (lifecycle
  // reset is on summon, see `quick-ask:opened` listener below).
  const shellRef = useRef<HTMLDivElement | null>(null);
  const [manualResized, setManualResized] = useState(false);
  useEffect(() => {
    if (manualResized) return;
    const el = shellRef.current;
    if (!el) return;
    let raf = 0;
    let last = -1;
    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (!entry) return;
      const measured =
        entry.borderBoxSize?.[0]?.blockSize ?? entry.contentRect.height;
      const next = Math.ceil(measured);
      if (next === last) return;
      last = next;
      if (raf) cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => {
        void quickAskSetHeight(next);
      });
    });
    observer.observe(el);
    return () => {
      if (raf) cancelAnimationFrame(raf);
      observer.disconnect();
    };
  }, [manualResized]);

  // Bottom-right resize handle. We can't lean on AppKit's native
  // resize loop here: tao 0.34's `drag_resize_window` returns
  // NotSupported on macOS, and `tauri-nspanel`'s `.borderless()`
  // clobbers the Resizable style mask the panel was created with
  // anyway. So we hand-roll the loop — mousedown captures the start
  // size + cursor, mousemove deltas drive `setSize` (rAF-throttled,
  // clamped to the same bounds the auto-fit path uses).
  //
  // Caveat: WKWebView only fires mousemove while the cursor is over
  // the webview. As long as the window grows with the cursor the
  // corner stays under it, but a fast outward fling can outrun the
  // resize and stall until the cursor re-enters. Good enough for now;
  // would need a Rust-side AppKit loop to fix completely.
  const onResizeHandleMouseDown = useCallback(
    (event: ReactMouseEvent<HTMLDivElement>) => {
      event.preventDefault();
      event.stopPropagation();
      setManualResized(true);

      const startX = event.screenX;
      const startY = event.screenY;
      const startW = window.innerWidth;
      const startH = window.innerHeight;
      const win = getCurrentWebviewWindow();

      let raf = 0;
      let pendingW = startW;
      let pendingH = startH;

      const apply = () => {
        raf = 0;
        void win.setSize(new LogicalSize(pendingW, pendingH));
      };

      const onMove = (e: MouseEvent) => {
        pendingW = Math.min(
          QUICK_ASK_MAX_WIDTH,
          Math.max(QUICK_ASK_MIN_WIDTH, startW + (e.screenX - startX)),
        );
        pendingH = Math.min(
          QUICK_ASK_MAX_HEIGHT,
          Math.max(QUICK_ASK_MIN_HEIGHT, startH + (e.screenY - startY)),
        );
        if (raf) return;
        raf = requestAnimationFrame(apply);
      };

      const onUp = () => {
        if (raf) {
          cancelAnimationFrame(raf);
          raf = 0;
        }
        document.removeEventListener("mousemove", onMove);
        document.removeEventListener("mouseup", onUp);
        // Flush the latest pending size — without this, releasing
        // between rAF ticks leaves the window one frame behind the
        // cursor's final position.
        void win.setSize(new LogicalSize(pendingW, pendingH));
      };

      document.addEventListener("mousemove", onMove);
      document.addEventListener("mouseup", onUp);
    },
    [],
  );

  const onOpenInApp = useCallback(async () => {
    if (!threadId) return;
    try {
      await quickAskOpenInApp(threadId);
    } catch (e) {
      setBootstrapError(String(e));
    }
  }, [threadId]);

  // 新建会话: type-to-create — drop the active thread and clear stream
  // state. The actual `chat_threads` row materializes on the next send
  // via `createThreadIfMissing` in the `useChatStream` options. Same
  // pattern the `/ask` page uses for its 「新建」 button.
  const onNewSession = useCallback(() => {
    setThreadId(null);
    setBootstrapError(null);
    setDraft("");
    setPickerOpen(false);
    stream.reset();
    requestAnimationFrame(() => inputRef.current?.focus());
  }, [stream]);

  // ⌘N / Ctrl+N — open a new session in-place (CO-86). Window-level
  // listener so the shortcut works regardless of which control owns
  // focus (textarea, send button, history picker). Captured here so
  // the system / WebKit default ⌘N ("new window") never reaches the
  // OS. Skipped during IME composition so picking a 拼音/かな/한글
  // candidate with Cmd held never resets the panel. Skipped while a
  // permission card is up — same reason ESC is blocked there.
  // `onNewSessionRef` lets the `[]`-deps listener always reach the
  // latest closure (which captures `stream`) without re-subscribing.
  const onNewSessionRef = useRef(onNewSession);
  onNewSessionRef.current = onNewSession;
  useEffect(() => {
    const handler = (event: globalThis.KeyboardEvent) => {
      if (event.isComposing || event.keyCode === 229) return;
      if (event.key !== "n" && event.key !== "N") return;
      if (!event.metaKey && !event.ctrlKey) return;
      if (event.shiftKey || event.altKey) return;
      event.preventDefault();
      if (pendingPermissionRef.current) return;
      onNewSessionRef.current();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  const onPickSession = useCallback((id: string) => {
    setThreadId(id);
    setBootstrapError(null);
    setDraft("");
    setPickerOpen(false);
    requestAnimationFrame(() => inputRef.current?.focus());
  }, []);

  // Trace what the FocusCard is actually showing to the user. Fires
  // only when the displayed app *changes*. We bounce it through a
  // Tauri IPC so the line lands in the same Rust tracing stream as
  // `foreground_monitor.focus_activated_emitted` etc. — reading two
  // separate logs (Rust + WebView devtools) for the same chain is a
  // pain.
  const displayedApp =
    attachedFocus?.app_name ??
    attachedFocus?.app_bundle_id ??
    skeleton?.app_name ??
    skeleton?.app_bundle_id ??
    null;
  const displayedSource = attachedFocus
    ? "focus"
    : skeleton
      ? "skeleton"
      : "none";
  useEffect(() => {
    quickAskLogDisplay(displayedApp, displayedSource).catch((e) => {
      // Surface the failure in WebView devtools so we can tell the
      // difference between "IPC failed" and "useEffect never ran".
      console.warn("[quick-ask] log_display ipc failed", e);
    });
  }, [displayedApp, displayedSource]);

  // Send is gated on attachedFocus (the orchestrator needs `frame_id`)
  // — no longer on threadId since the thread is created lazily on first
  // send.
  const canSend = draft.trim().length > 0 && !!attachedFocus;
  // The "在 App 中查看" button only makes sense once a thread exists,
  // i.e. after at least one send has gone through.
  const canOpenInApp = !!threadId;
  const titleText = useMemo(
    () =>
      firstUserMessage(stream.messages) ?? defaultTitleFor(attachedFocus, t),
    [stream.messages, attachedFocus, t],
  );

  // The swap chip is only meaningful when:
  //   - there's an attached anchor (otherwise nothing to "switch from"),
  //   - and the live focus has diverged to a different frame.
  // Bundle-id comparison would also work but frame_id is unique per
  // capture so it doubles as a cache key for the chip's identity.
  const liveDivergent =
    !!attachedFocus &&
    !!liveFocus &&
    liveFocus.frame_id !== attachedFocus.frame_id &&
    !liveFocus.excluded;

  return (
    <div
      className="quick-ask-shell"
      ref={shellRef}
      data-manual-resized={manualResized ? "true" : undefined}
      data-picker-open={pickerOpen ? "true" : undefined}
    >
      <div data-tauri-drag-region className="quick-ask-titlebar">
        <button
          type="button"
          className="quick-ask-close"
          onClick={() => void quickAskHide()}
          aria-label={t.quickAsk.closeAria}
          title={t.quickAsk.closeTitle}
        >
          <CloseIcon />
        </button>
        <div data-tauri-drag-region className="quick-ask-titlebar__title">
          {titleText}
        </div>
        <div className="quick-ask-titlebar__actions">
          <button
            type="button"
            className={
              "quick-ask-titlebar__btn" +
              (pickerOpen ? " quick-ask-titlebar__btn--active" : "")
            }
            onClick={() => setPickerOpen((v) => !v)}
            aria-label={t.quickAsk.historyAria}
            aria-pressed={pickerOpen}
            title={t.quickAsk.historyTitle}
          >
            <HistoryIcon />
          </button>
          <button
            type="button"
            className="quick-ask-titlebar__btn"
            onClick={onNewSession}
            aria-label={t.quickAsk.newAria}
            title={t.quickAsk.newTitle}
          >
            <PlusIcon />
          </button>
        </div>
        <button
          type="button"
          className="quick-ask-titlebar__hint"
          onClick={() => void quickAskHide()}
          title={t.quickAsk.closeTitle}
          aria-label={t.quickAsk.closeAria}
        >
          {t.quickAsk.titlebarHint} <kbd>⎋</kbd>
        </button>
      </div>

      <div className="quick-ask-body">
        {bootstrapError ? (
          <div className="quick-ask-error">{bootstrapError}</div>
        ) : null}

        {/* "Conversation region" — the flex-grow slot above the
            composer. The transcript lives here and the history picker
            (CO-47) overlays this region only, so the composer below
            stays visible and interactive regardless of picker state.
            `:empty` hides the wrapper when there's no transcript and no
            picker so we don't introduce a phantom gap. */}
        <div className="quick-ask-conversation">
          {stream.messages.length > 0 ? (
            <div ref={transcriptRef} className="quick-ask-transcript">
              {stream.messages.map((msg) => (
                <MessageBubble key={msg.id} message={msg} density="compact" />
              ))}
              {stream.error ? (
                <div className="quick-ask-error">{stream.error}</div>
              ) : null}
            </div>
          ) : null}

          {pickerOpen ? (
            <>
              {/* Click-outside dismiss. Scoped to the conversation region
                  so a click on the composer below doesn't steal focus or
                  toggle the picker — the user can keep drafting while
                  browsing history. */}
              <div
                className="quick-ask-picker-backdrop"
                onMouseDown={() => setPickerOpen(false)}
                aria-hidden="true"
              />
              <SessionPicker
                threads={threadsQuery.data ?? []}
                activeId={threadId}
                loading={threadsQuery.isLoading}
                onPick={onPickSession}
                onNew={onNewSession}
              />
            </>
          ) : null}
        </div>

        {permission.current ? (
          <QuickAskPermissionCard
            request={permission.current}
            onResolved={permission.dequeue}
          />
        ) : null}

        <div className="quick-ask-composer">
          <FocusCard
            focus={attachedFocus}
            skeleton={skeleton}
            loading={loadingFocus}
            liveFocus={liveDivergent ? liveFocus : null}
            onAdoptLive={liveDivergent ? adoptLiveFocus : undefined}
          />
          <div className="quick-ask-inputbar">
            <textarea
              ref={inputRef}
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={onKeyDown}
              placeholder={t.quickAsk.inputPlaceholder}
              rows={3}
              disabled={!focus}
            />
            <div className="quick-ask-inputbar__actions">
              <button
                type="button"
                className="quick-ask-iconbtn"
                onClick={onOpenInApp}
                disabled={!canOpenInApp}
                aria-label={t.quickAsk.openInAppAria}
                title={t.quickAsk.openInAppTitle}
              >
                <ExternalIcon />
              </button>
              <button
                type="button"
                className="quick-ask-iconbtn"
                disabled
                aria-label={t.quickAsk.micAria}
                title={t.quickAsk.micTitle}
              >
                <MicIcon />
              </button>
              <button
                type="button"
                className="quick-ask-iconbtn quick-ask-iconbtn--send"
                // While streaming, the send slot becomes a Stop button —
                // clicking signals `exec_agent_cancel` for the active
                // thread. The hook's `cancel` is a no-op when
                // !isStreaming, so this stays safe even with stale state.
                onClick={
                  stream.isStreaming ? () => void stream.cancel() : () => submit()
                }
                disabled={stream.isStreaming ? false : !canSend}
                aria-label={
                  stream.isStreaming
                    ? t.quickAsk.stopAria
                    : t.quickAsk.sendAria
                }
                title={
                  stream.isStreaming
                    ? t.quickAsk.stopTitle
                    : t.quickAsk.sendTitle
                }
              >
                {stream.isStreaming ? <StopIcon /> : <SendIcon />}
              </button>
            </div>
          </div>
        </div>
      </div>
      <div
        className="quick-ask-resize"
        onMouseDown={onResizeHandleMouseDown}
        aria-label={t.quickAsk.resizeAria}
        title={t.quickAsk.resizeTitle}
        role="presentation"
      >
        <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
          <path
            d="M11 6L6 11M11 1L1 11"
            stroke="currentColor"
            strokeWidth="1.25"
            strokeLinecap="round"
            fill="none"
          />
        </svg>
      </div>
    </div>
  );
}

/** Mirror of `format_focus_prompt`'s summary leg in the backend
 *  ([commands/exec_agent.rs](../../src-tauri/src/commands/exec_agent.rs)).
 *  Lives in the frontend because the FocusContext is held here; the
 *  backend just gets the rendered string. */
function focusSummary(focus: FocusContext): string {
  if (focus.excluded) {
    const app = focus.app_name ?? focus.app_bundle_id ?? "the current app";
    return `User is in ${app}. The app is on the privacy exclusion list, so the screen content is not available.`;
  }
  if (focus.empty) {
    return "User is on screen but the AX/OCR pipeline returned no text. The app may not expose accessibility, or permission is missing.";
  }
  const parts: string[] = [];
  if (focus.app_name) parts.push(`User is in ${focus.app_name}`);
  if (focus.window_title) parts.push(`Focused on: ${focus.window_title}`);
  if (focus.url) parts.push(`URL: ${focus.url}`);
  if (focus.adapter_name && focus.adapter_name !== "generic_ax") {
    parts.push(`(adapter: ${focus.adapter_name})`);
  }
  return parts.join(". ");
}

function firstUserMessage(
  messages: Array<{ role: "user" | "assistant" | "system"; content: string }>,
): string | null {
  const hit = messages.find((m) => m.role === "user");
  if (!hit) return null;
  const cleaned = hit.content.trim();
  if (!cleaned) return null;
  // Threads list shows ~32 chars; keep titles tight.
  return cleaned.length > 60 ? `${cleaned.slice(0, 60)}…` : cleaned;
}

function defaultTitleFor(
  focus: FocusContext | null,
  t: LocaleDict,
): string {
  if (!focus) return t.quickAsk.title;
  if (focus.window_title) return focus.window_title;
  if (focus.app_name) return t.quickAsk.titleWithApp(focus.app_name);
  return t.quickAsk.title;
}

/** Short label rendered on the user bubble's frame chip — "[App · 标题]"
 *  when both are present, falls back to whichever side exists. Truncates
 *  at 32 chars so a long URL doesn't overflow the panel. */
function shortFocusLabel(focus: FocusContext, t: LocaleDict): string {
  const app =
    focus.app_name ?? focus.app_bundle_id ?? null;
  const detail =
    focus.window_title?.trim() ||
    focus.url?.trim() ||
    null;
  const raw =
    app && detail
      ? `${app} · ${detail}`
      : (app ?? detail ?? t.quickAsk.focus.currentWindow);
  if (raw.length <= 32) return raw;
  return `${raw.slice(0, 32)}…`;
}

interface SessionPickerProps {
  threads: ChatThread[];
  activeId: string | null;
  loading: boolean;
  onPick: (id: string) => void;
  onNew: () => void;
}

function SessionPicker({
  threads,
  activeId,
  loading,
  onPick,
  onNew,
}: SessionPickerProps) {
  const { t } = useTranslation();
  return (
    <div className="quick-ask-picker">
      <button
        type="button"
        className="quick-ask-picker__row quick-ask-picker__row--new"
        onClick={onNew}
      >
        <span className="quick-ask-picker__icon">
          <PlusIcon />
        </span>
        <span className="quick-ask-picker__title">
          {t.quickAsk.picker.newSession}
        </span>
      </button>
      <div className="quick-ask-picker__divider" role="separator" />
      <div className="quick-ask-picker__list">
        {loading ? (
          <div className="quick-ask-picker__empty">
            {t.quickAsk.picker.loading}
          </div>
        ) : threads.length === 0 ? (
          <div className="quick-ask-picker__empty">
            {t.quickAsk.picker.empty}
          </div>
        ) : (
          threads.map((thread) => {
            const isActive = thread.id === activeId;
            return (
              <button
                key={thread.id}
                type="button"
                className={
                  "quick-ask-picker__row" +
                  (isActive ? " quick-ask-picker__row--active" : "")
                }
                onClick={() => onPick(thread.id)}
              >
                <span className="quick-ask-picker__title">
                  {thread.title?.trim() || t.quickAsk.picker.untitled}
                </span>
                <span className="quick-ask-picker__meta">
                  {formatRelative(thread.updated_at, t)}
                </span>
              </button>
            );
          })
        )}
      </div>
    </div>
  );
}

/** Compact relative timestamp for the session picker. Falls back to a
 *  locale date when the row is older than a week. */
function formatRelative(iso: string, t: LocaleDict): string {
  const ts = Date.parse(iso);
  if (Number.isNaN(ts)) return iso;
  const diff = Date.now() - ts;
  const minute = 60_000;
  const hour = 60 * minute;
  const day = 24 * hour;
  if (diff < minute) return t.quickAsk.relativeTime.justNow;
  if (diff < hour)
    return t.quickAsk.relativeTime.minutesAgo(Math.floor(diff / minute));
  if (diff < day)
    return t.quickAsk.relativeTime.hoursAgo(Math.floor(diff / hour));
  if (diff < 7 * day)
    return t.quickAsk.relativeTime.daysAgo(Math.floor(diff / day));
  try {
    return new Date(ts).toLocaleDateString();
  } catch {
    return iso;
  }
}

/* ---------- Inline icons (no extra dep) -------------------------------- */

function CloseIcon() {
  return (
    <svg width="11" height="11" viewBox="0 0 12 12" fill="none">
      <path
        d="M2.5 2.5l7 7M9.5 2.5l-7 7"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
      />
    </svg>
  );
}

function ExternalIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
      <path
        d="M14 4h6v6"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M20 4l-9 9"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M19 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1h5"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function MicIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
      <rect
        x="9"
        y="3"
        width="6"
        height="12"
        rx="3"
        stroke="currentColor"
        strokeWidth="1.6"
      />
      <path
        d="M5 11a7 7 0 0 0 14 0M12 18v3"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
      />
    </svg>
  );
}

function SendIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 16 16" fill="none">
      <path
        d="M8 13V3M3.5 7.5L8 3l4.5 4.5"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function StopIcon() {
  return (
    <svg width="12" height="12" viewBox="0 0 12 12" fill="currentColor">
      <rect x="2.5" y="2.5" width="7" height="7" rx="1.2" />
    </svg>
  );
}

function HistoryIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none">
      <path
        d="M3 12a9 9 0 1 0 3-6.7"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M3 4v4h4"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M12 7.5V12l3 1.8"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function PlusIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none">
      <path
        d="M12 5v14M5 12h14"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
      />
    </svg>
  );
}

