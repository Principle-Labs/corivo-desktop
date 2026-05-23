import { Outlet, createRoute } from "@tanstack/react-router";
import { OnboardingLayout } from "@/pages/onboarding/onboarding-layout";
import { Route as RootRoute } from "@/routes/__root";

export const Route = createRoute({
  getParentRoute: () => RootRoute,
  path: "/onboarding",
  component: () => (
    <OnboardingLayout>
      <Outlet />
    </OnboardingLayout>
  ),
});
