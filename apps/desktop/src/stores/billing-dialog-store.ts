import { useStore } from "zustand"
import { createStore } from "zustand/vanilla"

import { trackEvent } from "@/lib/tauri"

type BillingDialogState = {
  open: boolean
}

const store = createStore<BillingDialogState>(() => ({
  open: false,
}))

export function useBillingDialogOpen() {
  return useStore(store, (s) => s.open)
}

export function openBillingDialog() {
  // 用户每次打开付费 dialog 都打一个 Sentry event，做漏斗起点。
  // setBillingDialogOpen(true) 路径不上报，避免 dialog 内部的
  // controlled prop 来回切换被算成多次曝光。
  trackEvent("billing_dialog.opened")
  store.setState({ open: true })
}

export function closeBillingDialog() {
  store.setState({ open: false })
}

export function setBillingDialogOpen(open: boolean) {
  store.setState({ open })
}
