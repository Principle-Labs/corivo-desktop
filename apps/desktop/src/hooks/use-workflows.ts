import { useQuery } from "@tanstack/react-query";

import { workflowsList, workflowsListRuns } from "@/lib/tauri";

/**
 * Query-key conventions follow the strings already established by the
 * workflow listeners (`workflow-completed-listener.tsx`) and the
 * workflows page:
 *
 *   - `["workflows-list"]`       — definitions + schedules.
 *   - `["workflow-runs"]`        — recent runs across all slugs.
 *   - `["workflow-runs", slug]`  — runs filtered to one slug.
 *
 * The completed listener already calls `invalidateQueries` against the
 * unprefixed `["workflow-runs"]` form, so the sidebar repaint plays
 * back automatically as soon as a run finishes.
 */

export function useWorkflowsList() {
  return useQuery({
    queryKey: ["workflows-list"],
    queryFn: workflowsList,
    // Definitions change rarely; staying warm a few minutes is fine.
    staleTime: 60_000,
  });
}

/**
 * Recent runs across every slug. The sidebar reads this to weave
 * workflow run rows into the unified thread list. Default `limit=50`
 * mirrors the chat thread list's window so a busy day of automatic
 * runs doesn't drown out conversations *or* truncate halfway through.
 */
export function useWorkflowRuns(limit = 50) {
  return useQuery({
    queryKey: ["workflow-runs"],
    queryFn: () => workflowsListRuns({ limit }),
    staleTime: 5_000,
  });
}
