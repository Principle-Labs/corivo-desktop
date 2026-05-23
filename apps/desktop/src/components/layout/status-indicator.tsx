import { forwardRef, useEffect, useState } from "react";
import { ChevronUp } from "lucide-react";

import { useCaptureStatus } from "@/hooks/use-capture";
import { useTranslation } from "@/i18n";

import { ContextPopover } from "./context-popover";

/**
 * Sidebar bottom context-status row — the hero element of the bottom
 * cluster. Context-awareness state is the only always-on real-time
 * signal Corivo surfaces, so it earns visual weight that the other
 * bottom rows (account, hint) deliberately don't.
 *
 * Design intent (the de-anxiety pass):
 *   - The dot stays amber for "context enabled" but no longer pulses.
 *     A heartbeat animation reads as "Corivo is doing something to you"
 *     in a privacy-sensitive surface; the static dot reads as "ready".
 *   - Copy reads "Context enabled" / "Paused" — not "capturing" or
 *     "sharing". The user owns the context; we don't take it from them.
 *   - One click no longer flips the pipeline on/off. That was too
 *     blunt for a privacy surface. Instead, clicking opens the
 *     `<ContextPopover>` (pause durations, granular delete, manage).
 *
 * The button is forwarded into `PopoverTrigger asChild`, so Radix
 * attaches the click + a11y wiring without us re-implementing it.
 */
export function StatusIndicator() {
  return (
    <ContextPopover>
      <StatusIndicatorButton />
    </ContextPopover>
  );
}

const StatusIndicatorButton = forwardRef<
  HTMLButtonElement,
  React.ButtonHTMLAttributes<HTMLButtonElement>
>(function StatusIndicatorButton(props, ref) {
  const { t } = useTranslation();
  const { data: status } = useCaptureStatus();
  const remaining = usePausedRemaining(status?.paused_until ?? null);

  const phase = status?.phase ?? "stopped";
  const isRunning = phase === "running";
  const isPaused = !!status?.paused_until;

  const dotClass = isRunning
    ? "bg-[var(--corivo-amber)]"
    : "bg-muted-foreground/40";

  const label = isPaused
    ? remaining
      ? t.status.pausedFor(remaining)
      : t.status.stopped
    : isRunning
      ? t.status.capturing
      : t.status.disabled;

  // Running gets a faint amber wash. Paused/disabled fall back to ghost.
  const containerClass = isRunning && !isPaused
    ? "bg-[color-mix(in_oklab,var(--corivo-amber)_4%,transparent)] hover:bg-[color-mix(in_oklab,var(--corivo-amber)_8%,transparent)]"
    : "hover:bg-muted";

  return (
    <button
      ref={ref}
      type="button"
      aria-label={label}
      {...props}
      className={`group mx-1 flex items-center gap-2 rounded-md px-2 py-1.5 text-left transition-colors ${containerClass}`}
    >
      <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${dotClass}`} />
      <span className="min-w-0 flex-1 truncate text-[12px] text-foreground/80">
        {label}
      </span>
      <ChevronUp className="h-3 w-3 shrink-0 text-muted-foreground transition-colors group-hover:text-foreground" />
    </button>
  );
});

/**
 * Subscribe to `paused_until` and produce a human-readable "X left"
 * string, refreshed every 30 seconds while the pause is active.
 * Returns null when there's no pause or it has already expired (the
 * pipeline auto-resumes in that case; we just don't render a stale
 * countdown).
 */
function usePausedRemaining(pausedUntil: string | null): string | null {
  const [tick, setTick] = useState(0);

  useEffect(() => {
    if (!pausedUntil) return;
    const interval = setInterval(() => setTick((n) => n + 1), 30 * 1000);
    return () => clearInterval(interval);
  }, [pausedUntil]);

  if (!pausedUntil) return null;
  // `tick` is read here so the effect's increments trigger a re-render
  // and the math re-runs against `Date.now()`.
  void tick;

  const target = Date.parse(pausedUntil);
  if (Number.isNaN(target)) return null;
  const diffMs = target - Date.now();
  if (diffMs <= 0) return null;

  const minutes = Math.round(diffMs / 60_000);
  if (minutes < 60) return `${Math.max(1, minutes)}m`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.round(hours / 24);
  return `${days}d`;
}
