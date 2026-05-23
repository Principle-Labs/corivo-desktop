import { createRouter } from "@tanstack/react-router"
import { Route as RootRoute } from "@/routes/__root"
import { Route as IndexRoute } from "@/routes/index"
import { Route as AskRoute } from "@/routes/ask"
import { Route as LoginRoute } from "@/routes/login"
import { Route as OnboardingRoute } from "@/routes/onboarding"
import { Route as OnboardingPermissionRoute } from "@/routes/onboarding.permission"
import { Route as OnboardingDemoRoute } from "@/routes/onboarding.demo"
import { Route as OnboardingShortcutRoute } from "@/routes/onboarding.shortcut"
import { Route as WorkflowsRoute } from "@/routes/workflows"

const routeTree = RootRoute.addChildren([
  IndexRoute,
  AskRoute,
  LoginRoute,
  WorkflowsRoute,
  // v3 onboarding (docs/design/onboarding-redesign-v2.html). Permission
  // first — single-column layout, per-row grant buttons (AX + Screen
  // Recording each have their own macOS dialog so they need separate
  // buttons). Demo second — static showcase of the product (mock PRD
  // + Quick Ask + Apple Notes feedback). Shortcut third — keyboard
  // hero teaching ⌥⌥; the "进入 Corivo" button completes onboarding.
  OnboardingRoute.addChildren([
    OnboardingPermissionRoute,
    OnboardingDemoRoute,
    OnboardingShortcutRoute,
  ]),
])

export const router = createRouter({
  routeTree,
})

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router
  }
}
