import { useQuery } from "@tanstack/react-query"
import { getCapabilities, type Capabilities } from "@/lib/tauri"

export const CAPABILITIES_QUERY_KEY = ["capabilities"] as const

/**
 * Read the build's cloud-capability flags. **The answer is stable for
 * the lifetime of this binary** — capabilities are decided at compile
 * time (`corivo-cloud` cargo feature) plus boot-time wiring success,
 * never at runtime. Hence the infinite `staleTime`: fetch once, cache
 * forever.
 *
 * Returns `undefined` only on the very first render before the IPC
 * resolves; treat that as "assume nothing is available" rather than
 * "things will populate later" — once the first render lands, the
 * answer doesn't change. The components that gate on auth / billing
 * /  connectors render an empty fragment in the undefined window.
 *
 * Usage:
 *   const caps = useCapabilities();
 *   if (!caps?.billing) return null;
 *   return <BillingDialog />;
 */
export function useCapabilities() {
  return useQuery({
    queryKey: CAPABILITIES_QUERY_KEY,
    queryFn: getCapabilities,
    // 编译期 + boot 期决定，一旦拿到就不变；不让 react-query 再去
    // 重新抓也就避免了 SettingsPage mount / unmount 之间反复刷的成本。
    staleTime: Infinity,
    gcTime: Infinity,
  })
}

/**
 * Synchronous default for code paths that can't await the IPC (e.g. an
 * eager guard during initial route construction). Returns all-false —
 * "assume nothing" — which is the conservative answer for an OSS-style
 * build. Real values arrive via `useCapabilities()` on the first
 * subsequent render.
 */
export const CAPABILITIES_FALLBACK: Capabilities = {
  auth: false,
  billing: false,
  modelsDirectory: false,
  connectors: false,
  telemetry: false,
  managedUpdater: false,
}
