import { createRoute, redirect } from "@tanstack/react-router";
import { Route as RootRoute } from "@/routes/__root";

/**
 * `/` redirects to `/ask` — the wedge surface (spec §十二).
 */
export const Route = createRoute({
  getParentRoute: () => RootRoute,
  path: "/",
  beforeLoad: () => {
    throw redirect({ to: "/ask" });
  },
  component: () => null,
});
