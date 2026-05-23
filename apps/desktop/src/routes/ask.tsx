import { createRoute } from "@tanstack/react-router";
import { Route as RootRoute } from "@/routes/__root";
import { AskPage } from "@/pages/ask/ask-page";

interface AskSearch {
  /** Optional deep-link target — set by Quick Ask's "在 App 中查看"
   *  flow (router.navigate after the `ask:open-thread` event in
   *  `app-boot.tsx`). When present, AskPage uses it instead of
   *  defaulting to the most recent thread. */
  threadId?: string;
}

export const Route = createRoute({
  getParentRoute: () => RootRoute,
  path: "/ask",
  validateSearch: (search: Record<string, unknown>): AskSearch => ({
    threadId:
      typeof search.threadId === "string" && search.threadId.length > 0
        ? search.threadId
        : undefined,
  }),
  component: AskPage,
});
