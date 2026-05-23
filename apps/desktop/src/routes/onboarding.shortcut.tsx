import { createRoute } from "@tanstack/react-router";
import { ShortcutStep } from "@/pages/onboarding/steps/shortcut-step";
import { Route as OnboardingRoute } from "@/routes/onboarding";

export const Route = createRoute({
  getParentRoute: () => OnboardingRoute,
  path: "/shortcut",
  component: ShortcutStep,
});
