import type { ReactNode } from "react"
import { useRouterState } from "@tanstack/react-router"
import { Toaster } from "@repo/ui/components/sonner"
import { BillingDialog } from "@/components/billing/billing-dialog"
import { FrameDrawer } from "@/components/layout/frame-drawer"
import { Sidebar } from "@/components/layout/sidebar"
import { TitleBar } from "@/components/layout/title-bar"
import { WorkflowCompletedListener } from "@/components/layout/workflow-completed-listener"
import { SettingsDialog } from "@/components/settings/settings-dialog"
import { useCapabilities } from "@/hooks/use-capabilities"
import { useThemeSync } from "@/hooks/use-theme-sync"
import { cn } from "@repo/ui/lib/utils"

type AppLayoutProps = {
  children: ReactNode
}

// Routes that want the full width of the content shell (no reading-width
// cap, no outer padding). /ask is the three-pane chat workspace; it
// clips badly under max-w-5xl.
const FULL_WIDTH_ROUTES = ["/ask"]

export function AppLayout({ children }: AppLayoutProps) {
  useThemeSync()
  const pathname = useRouterState({ select: (s) => s.location.pathname })
  const fullWidth = FULL_WIDTH_ROUTES.some((p) => pathname === p || pathname.startsWith(`${p}/`))
  // capabilities.billing=false（开源构建）时彻底不挂 BillingDialog —— 它
  // 内部依赖 BillingMe IPC + Stripe 流，挂着就会无意义地占 query cache。
  // 第一次渲染 `data` 还在 loading 时 useCapabilities() 返回 undefined，
  // 我们保守地把 dialog 也藏着；拿到结果后会自然 re-render。
  const { data: capabilities } = useCapabilities()
  return (
    <div className="relative flex h-screen flex-col overflow-hidden bg-background text-foreground antialiased">
      <TitleBar />
      <div className="flex min-h-0 flex-1">
        <Sidebar />
        <main
          data-testid="app-content-shell"
          className="relative mt-5 mr-5 min-h-0 min-w-0 flex-1 overflow-hidden rounded-t-[20px] border-t border-r border-l border-border bg-card shadow-[0_1px_0_rgba(255,255,255,0.6)_inset]"
        >
          <div
            className={cn(
              "relative h-full overflow-auto",
              fullWidth ? "w-full" : "mx-auto max-w-5xl px-8 py-7",
            )}
          >
            {children}
          </div>
        </main>
      </div>
      <Toaster />
      <WorkflowCompletedListener />
      <SettingsDialog />
      <FrameDrawer />
      {capabilities?.billing ? <BillingDialog /> : null}
    </div>
  )
}
