import { useStore } from "zustand"
import { createStore } from "zustand/vanilla"

/**
 * SettingsDialog section id space. Kept loose (just `string`) on
 * purpose so deep-link callers don't have to import a giant union —
 * the dialog coerces unknown ids back to "general" on read.
 */
export type SettingsSectionId = string

type SettingsDialogState = {
  open: boolean
  /** Section id to switch the sidebar to on next open. Cleared after
   *  the dialog applies it so subsequent re-opens don't re-jump. */
  requestedSection?: SettingsSectionId
  /** DOM id of a card inside the active section to scroll into view
   *  on next open. Mirrors `requestedSection` in lifecycle: set by the
   *  caller, cleared by the section after it scrolls. */
  requestedCardId?: string
}

const store = createStore<SettingsDialogState>(() => ({
  open: false,
}))

export function useSettingsDialogOpen() {
  return useStore(store, (s) => s.open)
}

export function useSettingsRequestedSection() {
  return useStore(store, (s) => s.requestedSection)
}

export function useSettingsRequestedCardId() {
  return useStore(store, (s) => s.requestedCardId)
}

/**
 * Open Settings. When `section` is set, the dialog jumps to that tab
 * on open; when `cardId` is set, the section scrolls that card into
 * view once mounted. Both fields are one-shot — they clear after the
 * UI consumes them, so the next manual open returns to the user's
 * last-active tab.
 */
export function openSettingsDialog(opts?: {
  section?: SettingsSectionId
  cardId?: string
}) {
  store.setState({
    open: true,
    requestedSection: opts?.section,
    requestedCardId: opts?.cardId,
  })
}

export function closeSettingsDialog() {
  store.setState({
    open: false,
    requestedSection: undefined,
    requestedCardId: undefined,
  })
}

export function setSettingsDialogOpen(open: boolean) {
  store.setState({ open })
}

/**
 * Consume + clear the requested section / card. Called by the dialog
 * (for section) and the section components (for cardId) right after
 * they apply the targeting, so the next render doesn't re-trigger.
 */
export function clearSettingsRequest(field: "section" | "card") {
  store.setState((prev) =>
    field === "section"
      ? { ...prev, requestedSection: undefined }
      : { ...prev, requestedCardId: undefined },
  )
}
