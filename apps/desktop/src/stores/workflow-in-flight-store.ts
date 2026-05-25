import { create } from "zustand"

/**
 * Tracks workflow runs that are currently executing in the sidecar.
 *
 * Driven entirely by `workflow:started` / `workflow:completed` Tauri
 * events from the Rust backend. We keep the state purely in-memory
 * because:
 *
 * 1. The events are best-effort signals for "show a spinner" — losing
 *    state on window close is fine. The persistent record is the
 *    `workflow_runs` row written when consume_output finishes.
 * 2. Each entry's natural lifetime is the duration of one sidecar
 *    turn (seconds to a couple of minutes); persisting it would just
 *    leak stale "running" markers if the app crashes mid-turn.
 *
 * Keyed by slug, not run_id, because the row in the workflows list is
 * indexed by slug and "is this workflow running right now" is the
 * question the UI asks. A future per-run-id viewer can extend this.
 */

interface InFlightEntry {
  slug: string
  runId: string
  name: string
  toastId: string | number | null
}

interface InFlightState {
  entries: Map<string, InFlightEntry>
  start: (entry: InFlightEntry) => void
  finish: (slug: string) => InFlightEntry | undefined
  isInFlight: (slug: string) => boolean
}

export const useWorkflowInFlightStore = create<InFlightState>((set, get) => ({
  entries: new Map(),
  start: (entry) => {
    set((s) => {
      const next = new Map(s.entries)
      next.set(entry.slug, entry)
      return { entries: next }
    })
  },
  finish: (slug) => {
    const existing = get().entries.get(slug)
    set((s) => {
      const next = new Map(s.entries)
      next.delete(slug)
      return { entries: next }
    })
    return existing
  },
  isInFlight: (slug) => get().entries.has(slug),
}))
