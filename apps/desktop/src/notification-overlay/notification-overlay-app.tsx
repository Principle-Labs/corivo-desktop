import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

/**
 * Notification overlay (v1700).
 *
 * Standalone Tauri window pinned to the active screen's top-right
 * corner. Renders one persistent card per `workflow:completed` event;
 * a new event replaces the current card. The card does NOT auto-hide
 * — it stays put until the user either clicks the × or clicks the
 * card body (which navigates to the run's chat thread, then dismisses).
 * Single-slot policy means a fresh event quietly replaces the prior
 * card; the prior run is never lost (still in `workflow_runs`).
 */

interface WorkflowCompletedEvent {
  slug: string;
  run_id: string;
  thread_id: string;
  name: string;
  summary: string;
  success: boolean;
}

const HIDE_ANIMATION_MS = 200;

export function NotificationOverlayApp() {
  const [event, setEvent] = useState<WorkflowCompletedEvent | null>(null);
  const [hiding, setHiding] = useState(false);
  // Per-card identifier so re-mounting on a new event restarts the
  // slide-in animation cleanly even if the slug repeats.
  const cardKeyRef = useRef(0);

  // Subscribe to workflow:completed events emitted globally by the
  // Rust scheduled_workflows::notify::dispatch.
  useEffect(() => {
    const window = getCurrentWebviewWindow();
    const unlistenPromise = window.listen<WorkflowCompletedEvent>(
      "workflow:completed",
      (incoming) => {
        cardKeyRef.current += 1;
        setHiding(false);
        setEvent(incoming.payload);
        void invoke("notification_overlay_show").catch(() => {
          // Best-effort; the window may not be ready on the very
          // first event after boot.
        });
      },
    );
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  const beginHide = useCallback(() => {
    setHiding(true);
    window.setTimeout(() => {
      setEvent(null);
      setHiding(false);
      void invoke("notification_overlay_dismiss").catch(() => {});
    }, HIDE_ANIMATION_MS);
  }, []);

  if (!event) return null;

  const openRun = () => {
    void invoke("notification_overlay_open_run", {
      threadId: event.thread_id,
    }).catch(() => {});
    beginHide();
  };

  return (
    <div
      key={cardKeyRef.current}
      className="notification-card"
      data-state={hiding ? "hiding" : "visible"}
      onClick={openRun}
      role="button"
      tabIndex={0}
    >
      <div className="notification-header">
        <span
          className="notification-status-dot"
          data-success={String(event.success)}
          aria-hidden="true"
        />
        <span className="notification-name">{event.name}</span>
        <button
          type="button"
          className="notification-close"
          aria-label="Close"
          onClick={(e) => {
            e.stopPropagation();
            beginHide();
          }}
        >
          ×
        </button>
      </div>
      <div className="notification-summary">{event.summary}</div>
    </div>
  );
}
