import { useEffect } from "react"
import { useQueryClient } from "@tanstack/react-query"
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow"

interface WorkflowCompletedEvent {
  slug: string
  run_id: string
  thread_id: string
  name: string
  summary: string
  success: boolean
}

/**
 * Main-window listener for `workflow:completed` (v1700).
 *
 * Only job: invalidate React Query caches so the `/workflows` list +
 * history pages repaint when a run finishes. The toast UI lives in
 * the separate `notification-overlay` window, which has its own
 * listener — we don't render any toast or banner here anymore.
 */
export function WorkflowCompletedListener() {
  const qc = useQueryClient()

  useEffect(() => {
    const window = getCurrentWebviewWindow()
    const unlistenPromise = window.listen<WorkflowCompletedEvent>(
      "workflow:completed",
      (event) => {
        const { slug } = event.payload
        void qc.invalidateQueries({ queryKey: ["workflows-list"] })
        void qc.invalidateQueries({ queryKey: ["workflow-runs", slug] })
        void qc.invalidateQueries({ queryKey: ["corivo-suggestions"] })
      },
    )
    return () => {
      void unlistenPromise.then((unlisten) => unlisten())
    }
  }, [qc])

  return null
}
