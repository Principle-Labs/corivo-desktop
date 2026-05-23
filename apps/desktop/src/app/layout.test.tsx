// @vitest-environment jsdom

import { act } from "react"
import { createRoot, type Root } from "react-dom/client"
import { afterEach, describe, expect, it, vi } from "vitest"
import { AppLayout } from "@/app/layout"

vi.mock("@/components/layout/title-bar", () => ({
  TitleBar: () => <div data-testid="app-title-bar">title</div>,
}))

vi.mock("@/components/layout/sidebar", () => ({
  Sidebar: () => <div data-testid="app-sidebar">sidebar</div>,
}))

vi.mock("@repo/ui/components/sonner", () => ({
  Toaster: () => null,
}))

vi.mock("@/components/layout/frame-drawer", () => ({
  FrameDrawer: () => null,
}))

vi.mock("@/components/settings/settings-dialog", () => ({
  SettingsDialog: () => null,
}))

vi.mock("@/components/billing/billing-dialog", () => ({
  BillingDialog: () => null,
}))

vi.mock("@/hooks/use-theme-sync", () => ({
  useThemeSync: () => undefined,
}))

// AppLayout now reads cloud capabilities to gate the BillingDialog mount.
// Without QueryClient在 test 里跑就会抛 — 直接 stub 成"all enabled"，
// 行为对齐 capability split 之前。
vi.mock("@/hooks/use-capabilities", () => ({
  useCapabilities: () => ({
    data: {
      auth: true,
      billing: true,
      modelsDirectory: true,
      connectors: true,
      telemetry: true,
      managedUpdater: true,
    },
  }),
  CAPABILITIES_FALLBACK: {
    auth: false,
    billing: false,
    modelsDirectory: false,
    connectors: false,
    telemetry: false,
    managedUpdater: false,
  },
}))

vi.mock("@tanstack/react-router", () => ({
  useRouterState: ({ select }: { select?: (s: unknown) => unknown } = {}) => {
    const state = { location: { pathname: "/" } }
    return select ? select(state) : state
  },
}))

let container: HTMLDivElement | null = null
let root: Root | null = null

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  container?.remove()
  container = null
  root = null
})

describe("AppLayout", () => {
  it("uses a flex shell with a flex body under the title bar", () => {
    container = document.createElement("div")
    document.body.appendChild(container)
    root = createRoot(container)

    act(() => {
      root?.render(
        <AppLayout>
          <div>content</div>
        </AppLayout>
      )
    })

    const shell = container.firstElementChild as HTMLDivElement | null
    const titleBar = container.querySelector("[data-testid='app-title-bar']")
    const body = titleBar?.nextElementSibling as HTMLDivElement | null

    expect(shell?.className).toContain("flex")
    expect(shell?.className).toContain("flex-col")
    expect(body?.className).toContain("flex")
    expect(body?.className).toContain("flex-1")
  })
})
