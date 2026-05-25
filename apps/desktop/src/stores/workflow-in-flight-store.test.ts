import { describe, expect, it } from "vitest"

import { useWorkflowInFlightStore } from "./workflow-in-flight-store"

/**
 * Regression: the sidebar selector used to call
 * `useWorkflowInFlightStore((s) => Array.from(s.entries.values()))`
 * which returns a fresh array on every selector tick. Combined with
 * zustand's `useSyncExternalStore` snapshot-equality check that
 * triggers an infinite render loop in React 18+ ("Maximum update
 * depth exceeded"). The hot-path components now select the `entries`
 * Map directly and derive the array via `useMemo`; this test guards
 * that selecting `entries` is stable across no-op state ticks.
 */
describe("workflow in-flight store", () => {
  it("returns the same entries Map reference when nothing changed", () => {
    const before = useWorkflowInFlightStore.getState().entries
    // Trigger a read that doesn't mutate (the selector pattern the
    // sidebar uses).
    const sampleA = useWorkflowInFlightStore
      .getState()
      .isInFlight("does-not-exist")
    expect(sampleA).toBe(false)
    const after = useWorkflowInFlightStore.getState().entries
    expect(after).toBe(before)
  })

  it("produces a new entries Map reference only on start/finish", () => {
    const initial = useWorkflowInFlightStore.getState().entries
    useWorkflowInFlightStore.getState().start({
      slug: "test-slug",
      runId: "run-1",
      name: "Test workflow",
      toastId: null,
    })
    const afterStart = useWorkflowInFlightStore.getState().entries
    expect(afterStart).not.toBe(initial)
    expect(afterStart.has("test-slug")).toBe(true)

    useWorkflowInFlightStore.getState().finish("test-slug")
    const afterFinish = useWorkflowInFlightStore.getState().entries
    expect(afterFinish).not.toBe(afterStart)
    expect(afterFinish.has("test-slug")).toBe(false)
  })
})
