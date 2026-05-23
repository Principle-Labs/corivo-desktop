import { useQuery, useQueryClient } from "@tanstack/react-query"
import { billingMe } from "@/lib/tauri"

export const BILLING_ME_QUERY_KEY = ["billing", "me"] as const

/**
 * Read the current user's billing snapshot — balance, allowed top-up
 * amounts, last payment row. Mounted in the sidebar UserCard (always
 * on, balance shown as a secondary line under the displayName) and
 * the BillingDialog (which forces a refetch on open).
 *
 * `staleTime` is intentionally short so that returning to the dialog
 * after a Stripe redirect picks up the fresh balance on first read,
 * but most renders within a session don't trigger an IPC.
 *
 * `enabled=false`（开源构建：capabilities.billing=false）时彻底跳过
 * 网络往返 + react-query 的 error state——返回的 `data` 直接是
 * undefined，组件用 nullish 兜底逻辑无缝降级。
 */
export function useBillingMe(options: { enabled?: boolean } = {}) {
  const { enabled = true } = options
  return useQuery({
    queryKey: BILLING_ME_QUERY_KEY,
    queryFn: billingMe,
    staleTime: 30_000,
    enabled,
  })
}

export function useInvalidateBilling() {
  const qc = useQueryClient()
  return () =>
    qc.invalidateQueries({ queryKey: BILLING_ME_QUERY_KEY })
}
