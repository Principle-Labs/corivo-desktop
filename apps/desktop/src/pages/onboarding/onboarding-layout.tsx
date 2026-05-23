import { useEffect, type ReactNode } from "react";
import { useRouterState } from "@tanstack/react-router";
import { Toaster } from "@repo/ui/components/sonner";
import { Mascot } from "@/components/brand/mascot";
import { TitleBar } from "@/components/layout/title-bar";
import { saveOnboardingStep } from "@/lib/tauri";
import { cn } from "@repo/ui/lib/utils";

// v3 onboarding (docs/design/onboarding-redesign-v2.html).
//   1. Permission — single-column, per-row grant buttons. Auto-
//      advances 800 ms after both grants land.
//   2. Demo       — static showcase (mock PRD + Quick Ask + Apple
//      Notes feedback). "继续" button advances.
//   3. Shortcut   — keyboard teaching ⌥⌥. "进入 Corivo" calls
//      mark_onboarding_completed and routes to "/".
const STEP_ORDER = [
  "/onboarding/permission",
  "/onboarding/demo",
  "/onboarding/shortcut",
] as const;

/**
 * Three-step onboarding shell.
 *
 * Top:    macOS title bar (handled by `<TitleBar/>`)
 * Header: brand lockup (mascot + "Corivo") · step dots · "N / 3"
 * Body:   step content, centered, max-w 800, with vertical breathing
 *         room. Demo step needs ~720 px wide for the browser frame,
 *         so we go a bit wider than the old 640.
 * Footer: belongs to each step. The layout passes children through
 *         untouched so per-step CTA placement stays clean.
 */
export function OnboardingLayout({ children }: { children: ReactNode }) {
  const { location } = useRouterState();
  const currentIndex = STEP_ORDER.indexOf(
    location.pathname as (typeof STEP_ORDER)[number],
  );
  const activeIndex = currentIndex >= 0 ? currentIndex : 0;

  // Persist the current step so a force-quit / restart resumes where
  // the user left off. We persist `permission` and `demo` — those are
  // explicit positions the user reached. `shortcut` is intentionally
  // NOT persisted: it's the last screen before completion, and on
  // force-quit we'd rather they re-see the demo than land cold on a
  // keyboard diagram with no context for why.
  const stepName = location.pathname.replace("/onboarding/", "");
  useEffect(() => {
    if (stepName && stepName !== "shortcut") {
      void saveOnboardingStep(stepName);
    }
  }, [stepName]);

  return (
    <div className="relative flex h-screen flex-col overflow-hidden bg-background text-foreground antialiased">
      <TitleBar />

      <header className="flex shrink-0 items-center gap-3 border-b border-border bg-background px-9 py-3">
        <div className="flex items-center gap-2">
          <Mascot size="xs" />
          <span className="font-display text-[13px] font-semibold leading-none tracking-[-0.01em] text-foreground">
            Corivo
          </span>
        </div>
        <div className="ml-auto flex items-center gap-2 font-mono text-[10.5px] tracking-[0.06em] text-muted-foreground">
          <ProgressDots activeIndex={activeIndex} count={STEP_ORDER.length} />
          <span className="ml-1">
            {activeIndex + 1} / {STEP_ORDER.length}
          </span>
        </div>
      </header>

      {/* Body — flex-1 so it fills the remaining vertical space; the
          inner wrapper uses h-full so each step receives the actual
          available height. Steps with short content (permission /
          shortcut) opt into `justify-center` themselves; the demo
          step uses the full height so its PRD frame can flex-grow
          to fit the window — that's the only way to avoid the
          "needs to scroll on default window size" pitfall. */}
      <main className="flex-1 overflow-auto">
        <div className="mx-auto flex h-full max-w-[800px] flex-col items-stretch px-8 py-6">
          {children}
        </div>
      </main>
      <Toaster />
    </div>
  );
}

/**
 * Progress dots — three small dots that share the foreground color
 * and differ only in opacity. Earlier revisions painted done /
 * active / future as black / amber / grey, which read like the
 * macOS traffic-light colors. Same hue + opacity ramp is calmer and
 * doesn't trigger that pattern match.
 */
function ProgressDots({
  activeIndex,
  count,
}: {
  activeIndex: number;
  count: number;
}) {
  return (
    <span className="flex items-center gap-1.5">
      {Array.from({ length: count }).map((_, i) => {
        const state =
          i < activeIndex ? "done" : i === activeIndex ? "active" : "future";
        return (
          <span
            key={i}
            className={cn(
              "h-1.5 w-1.5 rounded-full bg-foreground transition-opacity",
              state === "active" && "opacity-90",
              state === "done" && "opacity-30",
              state === "future" && "opacity-10",
            )}
          />
        );
      })}
    </span>
  );
}
