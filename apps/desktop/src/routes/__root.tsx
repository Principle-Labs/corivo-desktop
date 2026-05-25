import { Outlet, createRootRoute, redirect, useRouterState } from "@tanstack/react-router"
import { TanStackRouterDevtools } from "@tanstack/react-router-devtools"
import { AppLayout } from "@/app/layout"
import { authStatus, getCapabilities, type Capabilities } from "@/lib/tauri"

function isOnboardingPath(path: string): boolean {
  return path.startsWith("/onboarding")
}

export const Route = createRootRoute({
  // Per-route auth gate. Every in-app navigation re-checks the session
  // by calling `auth_status` (local Tauri IPC — config read, no
  // network). Without this, a logged-out window could still navigate
  // between routes by direct URL or any stray `router.navigate` —
  // `app-boot.tsx` only guards the very first boot. Throwing
  // `redirect` here prevents the destination route from ever
  // mounting; React components never get a chance to fire requests
  // with a stale or missing token.
  beforeLoad: async ({ location }) => {
    const path = location.pathname

    // OSS / no-cloud builds have `capabilities.auth = false` (every cloud
    // trait is a noop) — there is no notion of "logged in" to gate on, so
    // the guard must let every route through except `/login` itself. In
    // an OSS binary the login page is not just unnecessary; it is an
    // invalid state, because every login command is a noop/unavailable.
    let caps: Capabilities | null = null
    try {
      caps = await getCapabilities()
    } catch {}
    if (caps?.auth === false) {
      if (path === "/login") {
        throw redirect({ to: "/", replace: true })
      }
      return
    }

    if (path === "/login" || isOnboardingPath(path)) return

    let loggedIn = false
    try {
      const status = await authStatus()
      loggedIn = status.loggedIn
    } catch {
      // Backend not ready / IPC failure: fail closed — better to bounce
      // a launching app to /login than to render protected UI that'll
      // throw on its first invoke.
    }
    if (!loggedIn) {
      throw redirect({ to: "/login", replace: true })
    }
  },
  component: RootComponent,
})

function RootComponent() {
  const routerState = useRouterState()
  const path = routerState.location.pathname
  const chromeless = path.startsWith("/onboarding") || path === "/login"

  return chromeless ? (
    <>
      <Outlet />
      {import.meta.env.DEV ? <TanStackRouterDevtools position="bottom-right" /> : null}
    </>
  ) : (
    <AppLayout>
      <Outlet />
      {import.meta.env.DEV ? <TanStackRouterDevtools position="bottom-right" /> : null}
    </AppLayout>
  )
}
