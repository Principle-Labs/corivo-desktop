import { createRoute } from "@tanstack/react-router";
import { PermissionStep } from "@/pages/onboarding/steps/permission-step";
import { Route as OnboardingRoute } from "@/routes/onboarding";

export const Route = createRoute({
  getParentRoute: () => OnboardingRoute,
  path: "/permission",
  component: PermissionStep,
});
