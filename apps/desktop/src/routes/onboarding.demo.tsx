import { createRoute } from "@tanstack/react-router";
import { DemoStep } from "@/pages/onboarding/steps/demo-step";
import { Route as OnboardingRoute } from "@/routes/onboarding";

export const Route = createRoute({
  getParentRoute: () => OnboardingRoute,
  path: "/demo",
  component: DemoStep,
});
