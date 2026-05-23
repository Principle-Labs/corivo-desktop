import { createRoute } from "@tanstack/react-router";

import { LoginPage } from "@/pages/login/login-page";
import { Route as RootRoute } from "@/routes/__root";

export const Route = createRoute({
  getParentRoute: () => RootRoute,
  path: "/login",
  component: LoginPage,
});
