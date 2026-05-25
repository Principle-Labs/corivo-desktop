import { useEffect } from "react"
import { useQueryClient } from "@tanstack/react-query"
import { useRouter } from "@tanstack/react-router"
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow"
import { toast } from "sonner"

import { useTranslation } from "@/i18n"
import { fromInvokeError, workflowsCancelRun } from "@/lib/tauri"
import { useWorkflowInFlightStore } from "@/stores/workflow-in-flight-store"

interface WorkflowStartedEvent {
  slug: string
  run_id: string
  name: string
}

interface WorkflowCompletedEvent {
  slug: string
  run_id: string
  thread_id: string
  name: string
  summary: string
  success: boolean
}

/**
 * Window-scoped listener for `workflow:completed`. Mounts once in the
 * main window's app layout. Two side effects per event:
 *
 *   1. Fires a sonner toast — the in-app counterpart to the macOS
 *      banner. Clicking the toast routes to `/workflows` so the user
 *      can read the full output. The toast itself doesn't ack the run
 *      (acking should reflect actual reading, not just dismissing the
 *      transient surface).
 *   2. Invalidates React Query so any open `/workflows` page repaints
 *      with the fresh `last_run_at` / `last_status` / `next_run_at`
 *      values without a manual refetch.
 */
export function WorkflowCompletedListener() {
  const { t } = useTranslation()
  const qc = useQueryClient()
  const router = useRouter()
  const startInFlight = useWorkflowInFlightStore((s) => s.start)
  const finishInFlight = useWorkflowInFlightStore((s) => s.finish)

  useEffect(() => {
    const window = getCurrentWebviewWindow()
    const unlistenStartedPromise = window.listen<WorkflowStartedEvent>(
      "workflow:started",
      (event) => {
        const { name, slug, run_id } = event.payload
        const existing = useWorkflowInFlightStore.getState().entries.get(slug)
        if (existing) {
          // Frontend already marked this slug in-flight optimistically
          // when the user clicked Run Now. Don't double-toast or
          // double-mark — just upgrade the placeholder run_id to the
          // real one so the completed event can match it later.
          startInFlight({ ...existing, runId: run_id })
          void qc.invalidateQueries({ queryKey: ["workflows-list"] })
          return
        }
        // Cron-triggered (no prior optimistic claim) — create UI
        // mirroring the Run-Now path. Two buttons, intentionally
        // unequal in visual weight:
        //   * action「取消」  — DESTRUCTIVE. Interrupts the run via
        //     the cancel IPC. Styled with `text-destructive` per-toast
        //     so the red color warns the user before they click. Was
        //     previously rendered with the same neutral foreground as
        //     "收起", which made the two buttons indistinguishable
        //     and the destructive intent invisible (Nielsen "error
        //     prevention" violation).
        //   * cancel「收起」  — Safe. Just closes the toast UI; the
        //     run keeps going.
        const toastId: string | number = toast.loading(name, {
          description: t.workflows.toast.running,
          duration: Infinity,
          action: {
            label: t.workflows.list.cancel,
            onClick: () => {
              void workflowsCancelRun(slug).catch((error) =>
                toast.error(fromInvokeError(error)),
              )
              toast.dismiss(toastId)
            },
          },
          cancel: {
            label: t.workflows.toast.dismiss,
            onClick: () => toast.dismiss(toastId),
          },
          classNames: {
            // Override the global action-button color (foreground) so
            // THIS action reads as destructive. `hover:` keeps the
            // gentle bg tint, just shifts toward the destructive hue.
            actionButton:
              "!text-destructive hover:!bg-destructive/10 hover:!text-destructive",
          },
        })
        startInFlight({ slug, runId: run_id, name, toastId })
        void qc.invalidateQueries({ queryKey: ["workflows-list"] })
      },
    )

    const unlistenCompletedPromise = window.listen<WorkflowCompletedEvent>(
      "workflow:completed",
      (event) => {
        const { name, summary, slug, success } = event.payload
        const prev = finishInFlight(slug)
        // Dismiss the in-flight loading toast (if any) and create a
        // brand-new transient toast for the result. We tried reusing
        // the id originally — sonner kept the prior toast's `cancel`
        // ("收起") + `action` ("取消") slots alongside the new
        // `action` ("查看"), leaving up to three buttons in a result
        // toast that should only have one. Dismissing + recreating
        // gives a clean slate at the cost of a tiny cross-fade.
        if (prev?.toastId !== null && prev?.toastId !== undefined) {
          toast.dismiss(prev.toastId)
        }
        const toastOptions = {
          description: summary,
          action: {
            label: t.workflows.toast.openAction,
            onClick: () => {
              void router.navigate({ to: "/workflows" })
            },
          },
        }
        const showToast = success ? toast.success : toast.error
        showToast(name, toastOptions)
        void qc.invalidateQueries({ queryKey: ["workflows-list"] })
        void qc.invalidateQueries({ queryKey: ["workflow-runs", slug] })
        void qc.invalidateQueries({ queryKey: ["workflows-unread"] })
        void qc.invalidateQueries({ queryKey: ["corivo-suggestions"] })
      },
    )
    return () => {
      void unlistenStartedPromise.then((unlisten) => unlisten())
      void unlistenCompletedPromise.then((unlisten) => unlisten())
    }
  }, [
    qc,
    router,
    startInFlight,
    finishInFlight,
    t.workflows.toast.openAction,
    t.workflows.toast.running,
  ])

  return null
}
