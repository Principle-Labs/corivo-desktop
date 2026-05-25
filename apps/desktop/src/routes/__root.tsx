import { Outlet, createRootRoute, redirect, useRouterState } from "@tanstack/react-router"
import { TanStackRouterDevtools } from "@tanstack/react-router-devtools"
import { AppLayout } from "@/app/layout"
import { authStatus, getCapabilities } from "@/lib/tauri"

// Routes that work without a session: the login page itself, and the
// onboarding funnel (we don't gate onboarding because it predates the
// auth step and can be exited via `/login` from inside). Every other
// route requires `loggedIn`.
function isPublicPath(path: string): boolean {
  return path === "/login" || path.startsWith("/onboarding")
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
    if (isPublicPath(location.pathname)) return

    // OSS / no-cloud builds have `capabilities.auth = false` (every cloud
    // trait is a noop) — there is no notion of "logged in" to gate on, so
    // the guard must let every non-public route through. Check capability
    // before status; if the capability probe itself fails we fall through
    // to the auth check below and preserve the original fail-closed
    // behaviour for cloud builds with a broken IPC.
    try {
      const caps = await getCapabilities()
      if (!caps.auth) return
    } catch {}

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
