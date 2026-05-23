import { createRoute } from "@tanstack/react-router";

import { WorkflowsPage } from "@/pages/workflows/workflows-page";
import { Route as RootRoute } from "@/routes/__root";

/**
 * `/workflows` — "我的工作流" surface.
 *
 * v1 ships an empty-state list: once `skill_manage`-style agent tools
 * land, agent-crystalized routines and user-authored workflows will
 * appear here. External capability skills (lark / Claude / future MCP)
 * stay out of this page — they're configured under Settings → 技能.
 *
 * Filtering happens by `AvailableSkill.kind === "workflow"`; today the
 * scanner only produces `kind: "capability"` so the list is empty by
 * design.
 */
export const Route = createRoute({
  getParentRoute: () => RootRoute,
  path: "/workflows",
  component: WorkflowsPage,
});
