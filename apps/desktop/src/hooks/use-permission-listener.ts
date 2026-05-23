import { type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useCallback, useEffect, useState } from "react";

export interface PermissionRequest {
  id: string;
  /** Short human-readable action label (e.g. "run shell command", "Bash"). */
  action: string;
  /** Optional explanation the agent wrote for why approval is needed. */
  reason?: string;
  /** Optional structured args; rendered as JSON in the details block. */
  details?: Record<string, unknown>;
}

interface RawPayload {
  id: string;
  action: string;
  reason?: string | null;
  details?: Record<string, unknown> | null;
}

/**
 * Subscribe to `exec-agent:permission-request` events from the MCP
 * bridge and queue them for UI confirmation. The dialog renders one
 * request at a time; `dequeue` is called after the user replies via
 * `execAgentPermissionReply`.
 *
 * Scoped to the **current webview** — Rust uses `app.emit_to(label, …)`
 * to route the prompt to whichever window started the turn (main vs.
 * quick-ask). The default global `listen()` from `@tauri-apps/api/event`
 * has `target: { kind: "Any" }` and would still pick up events meant
 * for the *other* window, so we go through the webview-scoped listen.
 */
export function usePermissionListener() {
  const [queue, setQueue] = useState<PermissionRequest[]>([]);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;

    const webview = getCurrentWebviewWindow();
    webview
      .listen<RawPayload>("exec-agent:permission-request", (event) => {
        const { id, action, reason, details } = event.payload;
        setQueue((prev) => {
          if (prev.some((r) => r.id === id)) return prev;
          return [
            ...prev,
            {
              id,
              action,
              reason: reason ?? undefined,
              details: details ?? undefined,
            },
          ];
        });
      })
      .then((u) => {
        if (cancelled) {
          u();
        } else {
          unlisten = u;
        }
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const dequeue = useCallback((id: string) => {
    setQueue((prev) => prev.filter((r) => r.id !== id));
  }, []);

  return { current: queue[0] ?? null, queueDepth: queue.length, dequeue };
}
